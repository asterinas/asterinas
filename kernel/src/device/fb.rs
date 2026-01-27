// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_framebuffer::{ColorMapEntry, FRAMEBUFFER, FrameBuffer, MAX_CMAP_SIZE, PixelFormat};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::mm::{HasPaddr, HasSize, VmIo, io_util::HasVmReaderWriter};

use super::registry::char;
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::{RawIoctl, dispatch_ioctl},
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
#[padding_struct]
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

/// Framebuffer colormap structure for userspace communication; `struct fb_cmap` in Linux.
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

mod ioctl_defs {
    use super::{FbCmapUser, FbFixScreenInfo, FbVarScreenInfo};
    use crate::util::ioctl::{InData, InOutData, NoData, OutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/fb.h#L13-L38>

    pub(super) type GetVarScreenInfo = ioc!(FBIOGET_VSCREENINFO, 0x4600, OutData<FbVarScreenInfo>);
    pub(super) type PutVarScreenInfo = ioc!(FBIOPUT_VSCREENINFO, 0x4601, InOutData<FbVarScreenInfo>);
    pub(super) type GetFixScreenInfo = ioc!(FBIOGET_FSCREENINFO, 0x4602, OutData<FbFixScreenInfo>);
    pub(super) type GetColorMap      = ioc!(FBIOGETCMAP,         0x4604, InData<FbCmapUser>);
    pub(super) type PutColorMap      = ioc!(FBIOPUTCMAP,         0x4605, InData<FbCmapUser>);

    // `NoData` is used below because they're not supported by efifb.
    pub(super) type PanDisplay       = ioc!(FBIOPAN_DISPLAY,     0x4606, NoData);
    pub(super) type Blank            = ioc!(FBIOBLANK,           0x4611, NoData);
}

impl Device for Fb {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux: major 29, minor 0
        DeviceId::new(MajorId::new(29), MinorId::new(0))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("fb0".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        let Some(framebuffer) = FRAMEBUFFER.get() else {
            return Err(Error::with_message(
                Errno::ENODEV,
                "the framebuffer device is not present",
            ));
        };
        let framebuffer = framebuffer.clone();
        Ok(Box::new(FbHandle { framebuffer }))
    }
}

impl FbHandle {
    /// Reads an array of `u16` color map values from userspace.
    fn read_color_maps_from_user(addr: usize, data: &mut [u16]) -> Result<()> {
        for (i, item) in data.iter_mut().enumerate() {
            let user_addr = addr + i * size_of::<u16>();
            *item = current_userspace!().read_val(user_addr)?;
        }
        Ok(())
    }

    /// Writes an array of `u16` color map values to userspace.
    fn write_color_maps_to_user(addr: usize, data: &[u16]) -> Result<()> {
        for (i, &value) in data.iter().enumerate() {
            let user_addr = addr + i * size_of::<u16>();
            current_userspace!().write_val(user_addr, &value)?;
        }
        Ok(())
    }

    /// Collects the information in the [`FbVarScreenInfo`].
    fn collect_var_screen_info(&self) -> FbVarScreenInfo {
        /// Default pixel clock calculation for efifb compatibility
        const DEFAULT_PIXEL_CLOCK_DIVISOR: u32 = 10_000_000;

        /// Default timing parameters for efifb compatibility
        const DEFAULT_RIGHT_MARGIN: u32 = 32;
        const DEFAULT_UPPER_MARGIN: u32 = 16;
        const DEFAULT_LOWER_MARGIN: u32 = 4;
        const DEFAULT_VSYNC_LEN: u32 = 4;

        let pixel_format = self.framebuffer.pixel_format();
        let (red, green, blue, transp) = FbBitfield::from_pixel_format(pixel_format);

        FbVarScreenInfo {
            xres: self.framebuffer.width() as u32,
            yres: self.framebuffer.height() as u32,
            xres_virtual: self.framebuffer.width() as u32,
            yres_virtual: self.framebuffer.height() as u32,
            bits_per_pixel: (8 * pixel_format.nbytes()) as u32,
            red,
            green,
            blue,
            transp,
            pixclock: DEFAULT_PIXEL_CLOCK_DIVISOR / self.framebuffer.width() as u32 * 1000
                / self.framebuffer.height() as u32,
            left_margin: (self.framebuffer.width() as u32 / 8) & 0xf8,
            right_margin: DEFAULT_RIGHT_MARGIN,
            upper_margin: DEFAULT_UPPER_MARGIN,
            lower_margin: DEFAULT_LOWER_MARGIN,
            vsync_len: DEFAULT_VSYNC_LEN,
            hsync_len: (self.framebuffer.width() as u32 / 8) & 0xf8,
            ..Default::default()
        }
    }

    /// Collects the information in the [`FbFixScreenInfo`].
    fn collect_fix_screen_info(&self) -> FbFixScreenInfo {
        FbFixScreenInfo {
            smem_start: self.framebuffer.io_mem().paddr() as u64,
            smem_len: self.framebuffer.io_mem().size() as u32,
            line_length: self.framebuffer.line_size() as u32,
            ..Default::default()
        }
    }

    /// Handles the [`ioctl_defs::GetColorMap`] ioctl command.
    ///
    /// Arguments:
    ///  - Input: [`FbCmapUser`] (specifying the range).
    ///  - Output: [`FbCmapUser`] (filled with color palette data).
    fn handle_get_cmap(&self, cmap_user: &FbCmapUser) -> Result<()> {
        if cmap_user.len == 0 {
            return Ok(());
        }

        let start = cmap_user.start as usize;
        let len = cmap_user.len as usize;

        // Get color map entries from framebuffer
        let entries = self.framebuffer.get_color_map(start, len).ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the color map index is out of bounds")
        })?;

        // Extract color channels and write to userspace
        let red: Vec<u16> = entries.iter().map(|e| e.red).collect();
        let green: Vec<u16> = entries.iter().map(|e| e.green).collect();
        let blue: Vec<u16> = entries.iter().map(|e| e.blue).collect();
        let transp: Vec<u16> = entries.iter().map(|e| e.transp).collect();

        Self::write_color_maps_to_user(cmap_user.red, &red)?;
        Self::write_color_maps_to_user(cmap_user.green, &green)?;
        Self::write_color_maps_to_user(cmap_user.blue, &blue)?;
        if cmap_user.transp != 0 {
            Self::write_color_maps_to_user(cmap_user.transp, &transp)?;
        }

        Ok(())
    }

    /// Handles the [`ioctl_defs::PutColorMap`] ioctl command.
    ///
    /// Arguments:
    ///  - Input: [`FbCmapUser`] (with color palette data).
    ///  - Output: None.
    fn handle_set_cmap(&self, cmap_user: &FbCmapUser) -> Result<()> {
        if cmap_user.len == 0 {
            return Ok(());
        }

        let start = cmap_user.start as usize;
        let len = cmap_user.len as usize;

        // Check the size to prevent excessive memory allocation
        if start > MAX_CMAP_SIZE || len > MAX_CMAP_SIZE - start {
            return_errno_with_message!(
                Errno::EINVAL,
                "the color map range exceeds its maximum size"
            );
        }

        // Read color data from userspace
        let mut red = vec![0u16; len];
        let mut green = vec![0u16; len];
        let mut blue = vec![0u16; len];
        let mut transp = vec![0u16; len];

        Self::read_color_maps_from_user(cmap_user.red, &mut red)?;
        Self::read_color_maps_from_user(cmap_user.green, &mut green)?;
        Self::read_color_maps_from_user(cmap_user.blue, &mut blue)?;
        if cmap_user.transp != 0 {
            Self::read_color_maps_from_user(cmap_user.transp, &mut transp)?;
        }

        // Build color map entries
        let entries: Vec<ColorMapEntry> = (0..len)
            .map(|i| ColorMapEntry {
                red: red[i],
                green: green[i],
                blue: blue[i],
                transp: transp[i],
            })
            .collect();

        // Set color map entries in framebuffer
        self.framebuffer.set_color_map(start, &entries)?;

        Ok(())
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

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetVarScreenInfo => {
                cmd.write(&self.collect_var_screen_info())?;
                Ok(0)
            }
            cmd @ PutVarScreenInfo => {
                // EFI framebuffers do not support changing settings. Linux
                // will return the old settings to user space and succeed.
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/video/fbdev/core/fbmem.c#L276-L279>.
                cmd.write(&self.collect_var_screen_info())?;
                Ok(0)
            }
            cmd @ GetFixScreenInfo => {
                cmd.write(&self.collect_fix_screen_info())?;
                Ok(0)
            }
            cmd @ GetColorMap => {
                self.handle_get_cmap(&cmd.read()?)?;
                Ok(0)
            }
            cmd @ PutColorMap => {
                self.handle_set_cmap(&cmd.read()?)?;
                Ok(0)
            }
            PanDisplay | Blank => {
                // These commands are not supported by efifb.
                // We return errors according to the Linux behavior.
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the ioctl command is not supported by efifb devices"
                )
            }
            _ => {
                log::debug!(
                    "the ioctl command {:#x} is unknown for framebuffer devices",
                    raw_ioctl.cmd()
                );
                return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown");
            }
        })
    }
}

pub(super) fn init_in_first_kthread() {
    if FRAMEBUFFER.get().is_none() {
        return;
    }

    char::register(Arc::new(Fb)).expect("failed to register framebuffer char device");
}
