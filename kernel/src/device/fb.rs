// SPDX-License-Identifier: MPL-2.0

use core::mem::size_of;

use aster_framebuffer::{FbBitfield, FrameBuffer, FRAMEBUFFER};
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

/// Maximum number of pseudo palette entries (for efifb compatibility)
const PSEUDO_PALETTE_SIZE: usize = 16;

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

/// Framebuffer colormap structure for userspace communication.
///
/// This structure is aligned with Linux's `fb_cmap_user` for compatibility.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct FbCmapUser {
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
    /// Reads an array of u16 values from userspace
    pub fn read_color_maps_from_user(addr: usize, data: &mut [u16]) -> Result<()> {
        if addr == 0 {
            return Ok(());
        }
        for (i, item) in data.iter_mut().enumerate() {
            let user_addr = addr + i * size_of::<u16>();
            *item = current_userspace!().read_val(user_addr)?;
        }
        Ok(())
    }

    /// Writes an array of u16 values to userspace
    pub fn write_color_maps_to_user(addr: usize, data: &[u16]) -> Result<()> {
        if addr == 0 {
            return Ok(());
        }
        for (i, &value) in data.iter().enumerate() {
            let user_addr = addr + i * size_of::<u16>();
            current_userspace!().write_val(user_addr, &value)?;
        }
        Ok(())
    }
    /// Handles the [`IoctlCmd::GETVSCREENINFO`] ioctl command.
    /// Takes no input and outputs a [`FbVarScreenInfo`] structure filled with current screen settings.
    fn handle_get_var_screen_info(&self, arg: usize) -> Result<i32> {
        let pixel_format = self.framebuffer.pixel_format();
        let (red, green, blue, transp) = FrameBuffer::pixel_format_to_bitfields(pixel_format);

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
    /// Takes no input and outputs a [`FbFixScreenInfo`] structure with hardware-specific information.
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

    /// Handles the [`IoctlCmd::GETCMAP`] ioctl command.
    /// Takes a [`aster_framebuffer::FbCmap`] structure specifying the range as input and outputs the same structure filled with color palette data.
    fn handle_get_cmap(&self, arg: usize) -> Result<i32> {
        let cmap_user: FbCmapUser = current_userspace!().read_val(arg)?;

        if cmap_user.len == 0 {
            return Ok(0);
        }

        let cmap = self.framebuffer.cmap().lock();

        // Check bounds
        let start = cmap_user.start as usize;
        let len = cmap_user.len as usize;

        if start >= cmap.red.len() || start + len > cmap.red.len() {
            return_errno_with_message!(Errno::EINVAL, "colormap index out of bounds");
        }

        // Copy color data to userspace
        let red_slice = &cmap.red[start..start + len];
        Self::write_color_maps_to_user(cmap_user.red, red_slice)?;

        let green_slice = &cmap.green[start..start + len];
        Self::write_color_maps_to_user(cmap_user.green, green_slice)?;

        let blue_slice = &cmap.blue[start..start + len];
        Self::write_color_maps_to_user(cmap_user.blue, blue_slice)?;

        if let Some(ref transp) = cmap.transp {
            let transp_slice = &transp[start..start + len];
            Self::write_color_maps_to_user(cmap_user.transp, transp_slice)?;
        }

        Ok(0)
    }

    /// Handles the [`IoctlCmd::PUTCMAP`] ioctl command.
    /// Takes a [`aster_framebuffer::FbCmap`] structure with color palette data as input and produces no output.
    fn handle_set_cmap(&self, arg: usize) -> Result<i32> {
        let cmap_user: FbCmapUser = current_userspace!().read_val(arg)?;

        if cmap_user.len == 0 {
            return Ok(0);
        }

        // Size check to prevent excessive memory allocation
        if cmap_user.len > 256 {
            return_errno_with_message!(Errno::EINVAL, "colormap too large");
        }

        let start = cmap_user.start as usize;
        let len = cmap_user.len as usize;

        // Update the colormap directly
        let mut cmap = self.framebuffer.cmap().lock();

        // Ensure the colormap has enough space
        let required_len = start + len;
        if cmap.red.len() < required_len {
            cmap.red.resize(required_len, 0);
            cmap.green.resize(required_len, 0);
            cmap.blue.resize(required_len, 0);
            if cmap_user.transp != 0 {
                if cmap.transp.is_none() {
                    cmap.transp = Some(alloc::vec![0u16; required_len]);
                } else if let Some(ref mut transp) = cmap.transp {
                    transp.resize(required_len, 0);
                }
            }
            cmap.len = required_len as u32;
        }

        // Read color data directly into colormap
        Self::read_color_maps_from_user(cmap_user.red, &mut cmap.red[start..start + len])?;
        Self::read_color_maps_from_user(cmap_user.green, &mut cmap.green[start..start + len])?;
        Self::read_color_maps_from_user(cmap_user.blue, &mut cmap.blue[start..start + len])?;

        if cmap_user.transp != 0 {
            if let Some(ref mut transp) = cmap.transp {
                Self::read_color_maps_from_user(cmap_user.transp, &mut transp[start..start + len])?;
            }
        }

        // Update pseudo palette entries
        for i in 0..len.min(PSEUDO_PALETTE_SIZE) {
            let idx = start + i;
            if idx < PSEUDO_PALETTE_SIZE {
                let transp_val = cmap.transp.as_ref().map(|t| t[idx]).unwrap_or(0);
                let _ = self.framebuffer.set_color_regs(
                    idx as u32,
                    cmap.red[idx],
                    cmap.green[idx],
                    cmap.blue[idx],
                    transp_val,
                    cmap.len,
                );
            }
        }

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
            IoctlCmd::GETCMAP => self.handle_get_cmap(arg),
            IoctlCmd::PUTCMAP => self.handle_set_cmap(arg),
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
