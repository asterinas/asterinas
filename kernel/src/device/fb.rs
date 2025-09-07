// SPDX-License-Identifier: MPL-2.0

use aster_framebuffer::{FrameBuffer, PixelFormat, FRAMEBUFFER};
use ostd::{
    mm::{HasPaddr, HasSize, VmIo},
    Pod,
};

use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        file_handle::Mappable,
        inode_handle::FileIo,
        utils::IoctlCmd,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Default pixel clock calculation for efifb compatibility
const DEFAULT_PIXEL_CLOCK_DIVISOR: u32 = 10_000_000;

/// Default timing parameters for efifb compatibility
const DEFAULT_RIGHT_MARGIN: u32 = 32;
const DEFAULT_UPPER_MARGIN: u32 = 16;
const DEFAULT_LOWER_MARGIN: u32 = 4;
const DEFAULT_VSYNC_LEN: u32 = 4;

pub struct Fb;

pub struct FbHandle {
    framebuffer: Arc<FrameBuffer>,
    offset: Mutex<usize>,
}

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

impl Device for Fb {
    fn type_(&self) -> DeviceType {
        DeviceType::Misc
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(29, 0)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let framebuffer = FRAMEBUFFER
            .get()
            .cloned()
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "there is no framebuffer device"))?;

        let handle = FbHandle {
            framebuffer,
            offset: Mutex::new(0),
        };

        Ok(Some(Arc::new(handle)))
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
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn mappable(&self) -> Result<Mappable> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EBADF, "device not opened");
    }
}

impl FbHandle {
    /// Converts pixel format to framebuffer bitfields
    fn pixel_format_to_bitfields(
        pixel_format: PixelFormat,
    ) -> (FbBitfield, FbBitfield, FbBitfield, FbBitfield) {
        match pixel_format {
            PixelFormat::Grayscale8 => {
                let bitfield = FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                };
                (bitfield, bitfield, bitfield, FbBitfield::default())
            }
            PixelFormat::Rgb565 => (
                FbBitfield {
                    offset: 11,
                    length: 5,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 5,
                    length: 6,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 5,
                    msb_right: 0,
                },
                FbBitfield::default(),
            ),
            PixelFormat::Rgb888 => (
                FbBitfield {
                    offset: 16,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 8,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield::default(),
            ),
            PixelFormat::BgrReserved => (
                FbBitfield {
                    offset: 16,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 8,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 0,
                    length: 8,
                    msb_right: 0,
                },
                FbBitfield {
                    offset: 24,
                    length: 8,
                    msb_right: 0,
                },
            ),
        }
    }

    /// Handles the [`IoctlCmd::GETVSCREENINFO`] ioctl command.
    fn handle_get_var_screen_info(&self, arg: usize) -> Result<i32> {
        let pixel_format = self.framebuffer.pixel_format();
        let (red, green, blue, transp) = Self::pixel_format_to_bitfields(pixel_format);

        let screen_info = FbVarScreenInfo {
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
        };

        current_userspace!().write_val(arg, &screen_info)?;
        Ok(0)
    }

    /// Handles the [`IoctlCmd::GETFSCREENINFO`] ioctl command.
    fn handle_get_fix_screen_info(&self, arg: usize) -> Result<i32> {
        let screen_info = FbFixScreenInfo {
            smem_start: self.framebuffer.io_mem().paddr() as u64,
            smem_len: (self.framebuffer.width()
                * self.framebuffer.height()
                * self.framebuffer.pixel_format().nbytes()) as u32,
            line_length: (self.framebuffer.width() * self.framebuffer.pixel_format().nbytes())
                as u32,
            ..Default::default()
        };

        current_userspace!().write_val(arg, &screen_info)?;
        Ok(0)
    }
}

impl FileIo for FbHandle {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let buffer_size = self.framebuffer.io_mem().size();
        let mut offset = self.offset.lock();

        if *offset >= buffer_size {
            return Ok(0); // EOF
        }

        let read_len = writer.avail().min(buffer_size - *offset);
        if read_len == 0 {
            return Ok(0);
        }

        // Read from the framebuffer at the current offset.
        // Limit the writer to avoid over-reading when the user buffer is
        // larger than the remaining framebuffer size.
        self.framebuffer
            .io_mem()
            .read(*offset, writer.limit(read_len))?;

        *offset += read_len;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let buffer_size = self.framebuffer.io_mem().size();
        let mut offset = self.offset.lock();

        if *offset >= buffer_size {
            return_errno_with_message!(Errno::ENOSPC, "write beyond framebuffer size");
        }

        let write_len = reader.remain().min(buffer_size - *offset);
        if write_len == 0 {
            return Ok(0);
        }

        // Write to the framebuffer at the current offset.
        // Limit the reader to avoid over-writing when the user buffer is
        // larger than the remaining framebuffer size.
        self.framebuffer
            .io_mem()
            .write(*offset, reader.limit(write_len))?;

        *offset += write_len;
        Ok(write_len)
    }

    fn mappable(&self) -> Result<Mappable> {
        let iomem = self.framebuffer.io_mem();
        Ok(Mappable::IoMem(iomem.clone()))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::GETVSCREENINFO => self.handle_get_var_screen_info(arg),
            IoctlCmd::GETFSCREENINFO => self.handle_get_fix_screen_info(arg),
            IoctlCmd::PUTVSCREENINFO => {
                // The framebuffer we are working with is initialized by UEFI
                //  services, which do not support changing **ANY** settings.
                //  Therefore, we simply copy the current settings back to
                //  userspace without making any changes. This behavior is
                //  consistent with Linux's efifb driver.
                self.handle_get_var_screen_info(arg)
            }
            IoctlCmd::GETCMAP => {
                log::debug!("Fb ioctl: Get color map");
                // TODO: Implement logic to get the color map
                Ok(0)
            }
            IoctlCmd::PUTCMAP => {
                log::debug!("Fb ioctl: Set color map");
                // TODO: Implement logic to set the color map
                Ok(0)
            }
            IoctlCmd::PANDISPLAY | IoctlCmd::FBIOBLANK => {
                // These commands are not supported by efifb.
                // We return errors according to the Linux behavior.
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the ioctl command is not supported by efifb devices"
                )
            }
            _ => {
                log::debug!(
                    "the ioctl command {:?} is not supported by framebuffer devices",
                    cmd
                );
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the ioctl command is not supported by framebuffer devices"
                )
            }
        }
    }
}

impl Pollable for FbHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
