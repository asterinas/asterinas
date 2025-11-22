// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_framebuffer::{ColorMapEntry, FrameBuffer, PixelFormat, FRAMEBUFFER, MAX_CMAP_SIZE};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{
    mm::{io_util::HasVmReaderWriter, HasPaddr, HasSize},
    Pod,
};

use super::char::{self, CharDevice, DevtmpfsName};
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

#[derive(Debug)]
struct Fb;

#[derive(Debug)]
struct FbHandle {
    framebuffer: Arc<FrameBuffer>,
}

/// Bitfields describing the color channel layout; `struct fb_bitfield` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/fb.h#L189>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Default)]
struct FbBitfield {
    /// Bit offset of the field
    pub offset: u32,
    /// Length of the field in bits
    pub length: u32,
    /// Most significant bit position (0 = left, 1 = right)
    pub msb_right: u32,
}

impl FbBitfield {
    /// Converts pixel format to framebuffer bitfields for Linux compatibility.
    #[rustfmt::skip]
    fn from_pixel_format(pixel_format: PixelFormat) -> (Self, Self, Self, Self) {
        match pixel_format {
            PixelFormat::Grayscale8 => (
                Self { offset: 0, length: 8, msb_right: 0 },
                Self { offset: 0, length: 8, msb_right: 0 },
                Self { offset: 0, length: 8, msb_right: 0 },
                Self::default(),
            ),
            PixelFormat::Rgb565 => (
                Self { offset: 11, length: 5, msb_right: 0 },
                Self { offset: 5, length: 6, msb_right: 0 },
                Self { offset: 0, length: 5, msb_right: 0 },
                Self::default(),
            ),
            PixelFormat::Rgb888 => (
                Self { offset: 16, length: 8, msb_right: 0 },
                Self { offset: 8, length: 8, msb_right: 0 },
                Self { offset: 0, length: 8, msb_right: 0 },
                Self::default(),
            ),
            PixelFormat::BgrReserved => (
                Self { offset: 16, length: 8, msb_right: 0 },
                Self { offset: 8, length: 8, msb_right: 0 },
                Self { offset: 0, length: 8, msb_right: 0 },
                Self { offset: 24, length: 8, msb_right: 0 },
            ),
        }
    }
}

/// Variable screen information for framebuffer devices; `struct fb_var_screeninfo` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/fb.h#L243>.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
struct FbVarScreenInfo {
    /// Visible resolution width
    pub xres: u32,
    /// Visible resolution height
    pub yres: u32,
    /// Virtual resolution width
    pub xres_virtual: u32,
    /// Virtual resolution height
    pub yres_virtual: u32,
    /// Offset from virtual to visible (horizontal)
    pub xoffset: u32,
    /// Offset from virtual to visible (vertical)
    pub yoffset: u32,
    /// Color depth in bits per pixel
    pub bits_per_pixel: u32,
    /// 0 = color, 1 = grayscale, >1 = FOURCC
    pub grayscale: u32,
    /// Red color bitfield in framebuffer memory
    pub red: FbBitfield,
    /// Green color bitfield in framebuffer memory
    pub green: FbBitfield,
    /// Blue color bitfield in framebuffer memory
    pub blue: FbBitfield,
    /// Transparency bitfield
    pub transp: FbBitfield,
    /// Non-standard pixel format indicator
    pub nonstd: u32,
    /// Activation control flags
    pub activate: u32,
    /// Height of display in millimeters
    pub height: u32,
    /// Width of display in millimeters
    pub width: u32,
    /// Acceleration capabilities (obsolete)
    pub accel_flags: u32,
    /// Pixel clock period in picoseconds
    pub pixclock: u32,
    /// Time from horizontal sync to picture
    pub left_margin: u32,
    /// Time from picture to horizontal sync
    pub right_margin: u32,
    /// Time from vertical sync to picture
    pub upper_margin: u32,
    /// Time from picture to vertical sync
    pub lower_margin: u32,
    /// Length of horizontal sync
    pub hsync_len: u32,
    /// Length of vertical sync
    pub vsync_len: u32,
    /// Synchronization flags
    pub sync: u32,
    /// Video mode flags
    pub vmode: u32,
    /// Screen rotation angle (counter-clockwise)
    pub rotate: u32,
    /// Colorspace for FOURCC-based modes
    pub colorspace: u32,
    /// Reserved for future compatibility
    pub reserved: [u32; 4],
}

/// Fixed screen information for framebuffer devices; `struct fb_fix_screeninfo` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/fb.h#L158>.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
struct FbFixScreenInfo {
    /// Identification string (e.g., "EFI VGA")
    pub id: [u8; 16],
    /// Start of framebuffer memory (physical address)
    pub smem_start: u64,
    /// Length of framebuffer memory in bytes
    pub smem_len: u32,
    /// Framebuffer type identifier
    pub type_: u32,
    /// Auxiliary type information (e.g., interleave)
    pub type_aux: u32,
    /// Visual type (mono, pseudo-color, true-color, etc.)
    pub visual: u32,
    /// Horizontal panning step size (0 = no panning)
    pub xpanstep: u16,
    /// Vertical panning step size (0 = no panning)
    pub ypanstep: u16,
    /// Y-axis wrapping step size (0 = no wrapping)
    pub ywrapstep: u16,
    /// Length of a screen line in bytes
    pub line_length: u32,
    /// Start of memory-mapped I/O (physical address)
    pub mmio_start: u64,
    /// Length of memory-mapped I/O region
    pub mmio_len: u32,
    /// Hardware acceleration type identifier
    pub accel: u32,
    /// Hardware capability flags
    pub capabilities: u16,
    /// Reserved for future compatibility
    pub reserved: [u16; 2],
}

/// Framebuffer colormap structure for userspace communication.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/fb.h#L283>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct FbCmapUser {
    /// Starting offset in colormap
    pub start: u32,
    /// Number of colormap entries
    pub len: u32,
    /// Pointer to red color values in userspace
    pub red: usize,
    /// Pointer to green color values in userspace
    pub green: usize,
    /// Pointer to blue color values in userspace
    pub blue: usize,
    /// Pointer to transparency values in userspace (may be null)
    pub transp: usize,
}

impl CharDevice for Fb {
    fn devtmpfs_name(&self) -> DevtmpfsName<'_> {
        DevtmpfsName::new("fb0", None)
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux: major 29, minor 0
        DeviceId::new(MajorId::new(29), MinorId::new(0))
    }

    fn open(&self) -> Result<Arc<dyn FileIo>> {
        let Some(framebuffer) = FRAMEBUFFER.get() else {
            return Err(Error::with_message(
                Errno::ENODEV,
                "the framebuffer device is not present",
            ));
        };
        let framebuffer = framebuffer.clone();
        Ok(Arc::new(FbHandle { framebuffer }))
    }
}

impl Pollable for FbHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for FbHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if !writer.has_avail() {
            return Ok(0);
        }

        let mut reader = self.framebuffer.io_mem().reader();

        if offset >= reader.remain() {
            return Ok(0);
        }
        reader.skip(offset);

        let mut reader = reader.to_fallible();
        let len = match reader.read_fallible(writer) {
            Ok(len) => len,
            Err((err, 0)) => return Err(err.into()),
            Err((_err, len)) => len,
        };

        Ok(len)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        let mut writer = self.framebuffer.io_mem().writer();
        if offset >= writer.avail() {
            return_errno_with_message!(
                Errno::ENOSPC,
                "the write offset is beyond the framebuffer size"
            );
        }
        writer.skip(offset);

        let mut writer = writer.to_fallible();
        let len = match writer.write_fallible(reader) {
            Ok(len) => len,
            Err((err, 0)) => return Err(err.into()),
            Err((_err, len)) => len,
        };

        Ok(len)
    }
}

impl FileIo for FbHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn mappable(&self) -> Result<Mappable> {
        let iomem = self.framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            _ => {
                log::debug!(
                    "the ioctl command {:?} is not supported by framebuffer devices",
                    cmd
                );
                return_errno_with_message!(
                    Errno::ENOTTY,
                    "the ioctl command is not supported by framebuffer devices"
                )
            }
        }
    }
}

pub(super) fn init_in_first_kthread() {
    if FRAMEBUFFER.get().is_none() {
        return;
    }

    char::register(Arc::new(Fb)).expect("failed to register framebuffer char device");
}
