// SPDX-License-Identifier: MPL-2.0

pub(crate) use aster_framebuffer::get_framebuffer_info;
use aster_framebuffer::FrameBuffer;

use super::*;
use crate::{
    events::IoEvents,
    fs::{file_handle::Mappable, inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};
pub struct Fb;

/// Variable screen information structure for framebuffer devices.
///
/// This structure is aligned with Linux's `fb_var_screeninfo` to maintain
/// compatibility with system call interfaces and userspace applications.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct FbVarScreenInfo {
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

/// Bitfield structure describing color channel layout.
///
/// This structure is aligned with Linux's `fb_bitfield` for system call compatibility.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Default)]
pub struct FbBitfield {
    /// Bit offset of the field
    pub offset: u32,
    /// Length of the field in bits
    pub length: u32,
    /// Most significant bit position (0 = left, 1 = right)
    pub msb_right: u32,
}

/// Fixed screen information structure for framebuffer devices.
///
/// This structure is aligned with Linux's `fb_fix_screeninfo` to maintain
/// compatibility with system call interfaces and userspace applications.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct FbFixScreenInfo {
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

impl Fb {
    /// Get the framebuffer instance or return an error if not initialized.
    fn get_framebuffer(&self) -> Result<Arc<FrameBuffer>> {
        get_framebuffer_info().ok_or_else(|| {
            Error::with_message(Errno::ENODEV, "Framebuffer has not been initialized")
        })
    }
}

impl Device for Fb {
    fn type_(&self) -> DeviceType {
        DeviceType::Misc
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(29, 0)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(Fb)))
    }
}

impl Pollable for Fb {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Fb {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        // Reading from framebuffer is not supported
        return_errno_with_message!(Errno::ENOSYS, "Fb: read is not supported");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        // Writing to the framebuffer device is not supported.
        return_errno_with_message!(Errno::EINVAL, "Writing to framebuffer is not supported");
    }

    fn mappable(&self) -> Result<Mappable> {
        let framebuffer = self.get_framebuffer()?;
        let iomem = framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        log::debug!("Fb ioctl: Unsupported command -> {:?}", cmd);
        return_errno!(Errno::EINVAL);
    }
}
