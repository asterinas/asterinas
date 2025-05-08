// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

pub(crate) use aster_framebuffer::get_framebuffer_info;
use ostd::Pod;

use super::*;
use crate::{
    current_userspace,
    events::IoEvents,
    fs::{file_handle::MemoryToMap, inode_handle::FileIo, utils::IoctlCmd},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct Fb;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct FbVarScreenInfo {
    pub xres: u32, // Visible resolution
    pub yres: u32,
    pub xres_virtual: u32, // Virtual resolution
    pub yres_virtual: u32,
    pub xoffset: u32, // Offset from virtual to visible
    pub yoffset: u32,
    pub bits_per_pixel: u32, // Guess what
    pub grayscale: u32,      // 0 = color, 1 = grayscale, >1 = FOURCC
    // Add other fields as needed
    pub red: FbBitfield,    // Bitfield in framebuffer memory if true color
    pub green: FbBitfield,  // Else only length is significant
    pub blue: FbBitfield,   // Bitfield in framebuffer memory if true color
    pub transp: FbBitfield, // Transparency
    pub nonstd: u32,        // Non-standard pixel format
    pub activate: u32,      // See FB_ACTIVATE_*
    pub height: u32,        // Height of picture in mm
    pub width: u32,         // Width of picture in mm
    pub accel_flags: u32,   // (OBSOLETE) see fb_info.flags
    pub pixclock: u32,      // Pixel clock in ps (pico seconds)
    pub left_margin: u32,   // Time from sync to picture
    pub right_margin: u32,  // Time from picture to sync
    pub upper_margin: u32,  // Time from sync to picture
    pub lower_margin: u32,
    pub hsync_len: u32,     // Length of horizontal sync
    pub vsync_len: u32,     // Length of vertical sync
    pub sync: u32,          // See FB_SYNC_*
    pub vmode: u32,         // See FB_VMODE_*
    pub rotate: u32,        // Angle we rotate counter-clockwise
    pub colorspace: u32,    // Colorspace for FOURCC-based modes
    pub reserved: [u32; 4], // Reserved for future compatibility
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Default)]
pub struct FbBitfield {
    pub offset: u32,
    pub length: u32,
    pub msb_right: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, Pod)]
pub struct FbFixScreenInfo {
    pub id: [u8; 16],       // Identification string, e.g., "TT Builtin"
    pub smem_start: u64,    // Start of framebuffer memory (physical address)
    pub smem_len: u32,      // Length of framebuffer memory
    pub type_: u32,         // See FB_TYPE_*
    pub type_aux: u32,      // Interleave for interleaved planes
    pub visual: u32,        // See FB_VISUAL_*
    pub xpanstep: u16,      // Zero if no hardware panning
    pub ypanstep: u16,      // Zero if no hardware panning
    pub ywrapstep: u16,     // Zero if no hardware ywrap
    pub line_length: u32,   // Length of a line in bytes
    pub mmio_start: u64,    // Start of Memory Mapped I/O (physical address)
    pub mmio_len: u32,      // Length of Memory Mapped I/O
    pub accel: u32,         // Indicate to driver which specific chip/card we have
    pub capabilities: u16,  // See FB_CAP_*
    pub reserved: [u16; 2], // Reserved for future compatibility
}

impl Device for Fb {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
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
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Fb {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        println!("Fb read");
        Ok(0)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        println!("Fb write");
        Ok(reader.remain())
    }

    fn mmap(&self) -> Result<MemoryToMap> {
        if let Some(framebuffer) = get_framebuffer_info() {
            let iomem = framebuffer.io_mem();
            Ok(MemoryToMap::IoMem(iomem.clone()))
        } else {
            return_errno_with_message!(Errno::ENODEV, "Framebuffer has not been initialized");
        }
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::GETVSCREENINFO => {
                println!("Fb ioctl: Get virtual screen info");

                // Use get_framebuffer_info to access the framebuffer
                if let Some(framebuffer_guard) = get_framebuffer_info() {
                    let framebuffer = &*framebuffer_guard; // Dereference the guard to access the FrameBuffer

                    // FIXME: On demand add more fields
                    let pixel_format = framebuffer.pixel_format();
                    let (red_bitfield, green_bitfield, blue_bitfield, transp_bitfield) =
                        match pixel_format {
                            aster_framebuffer::PixelFormat::Grayscale8 => {
                                // For grayscale, all color channels map to the same 8-bit value
                                let bitfield = FbBitfield {
                                    offset: 0,
                                    length: 8,
                                    msb_right: 0,
                                };
                                (
                                    bitfield,
                                    bitfield,
                                    bitfield,
                                    FbBitfield {
                                        offset: 0,
                                        length: 0,
                                        msb_right: 0,
                                    },
                                )
                            }
                            aster_framebuffer::PixelFormat::Rgb565 => {
                                (
                                    FbBitfield {
                                        offset: 11,
                                        length: 5,
                                        msb_right: 0,
                                    }, // Red: 5 bits at offset 11
                                    FbBitfield {
                                        offset: 5,
                                        length: 6,
                                        msb_right: 0,
                                    }, // Green: 6 bits at offset 5
                                    FbBitfield {
                                        offset: 0,
                                        length: 5,
                                        msb_right: 0,
                                    }, // Blue: 5 bits at offset 0
                                    FbBitfield {
                                        offset: 0,
                                        length: 0,
                                        msb_right: 0,
                                    }, // No transparency
                                )
                            }
                            aster_framebuffer::PixelFormat::Rgb888 => {
                                (
                                    FbBitfield {
                                        offset: 16,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Red: 8 bits at offset 16
                                    FbBitfield {
                                        offset: 8,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Green: 8 bits at offset 8
                                    FbBitfield {
                                        offset: 0,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Blue: 8 bits at offset 0
                                    FbBitfield {
                                        offset: 0,
                                        length: 0,
                                        msb_right: 0,
                                    }, // No transparency
                                )
                            }
                            aster_framebuffer::PixelFormat::BgrReserved => {
                                (
                                    FbBitfield {
                                        offset: 16,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Red: 8 bits at offset 16
                                    FbBitfield {
                                        offset: 8,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Green: 8 bits at offset 8
                                    FbBitfield {
                                        offset: 0,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Blue: 8 bits at offset 0
                                    FbBitfield {
                                        offset: 24,
                                        length: 8,
                                        msb_right: 0,
                                    }, // Reserved: 8 bits at offset 24
                                )
                            }
                        };

                    let screen_info = FbVarScreenInfo {
                        xres: framebuffer.width() as u32,
                        yres: framebuffer.height() as u32,
                        xres_virtual: framebuffer.width() as u32,
                        yres_virtual: framebuffer.height() as u32,
                        bits_per_pixel: (8 * pixel_format.nbytes()) as u32,
                        red: red_bitfield,
                        green: green_bitfield,
                        blue: blue_bitfield,
                        transp: transp_bitfield,
                        // Data are set according to the linux efifb driver
                        pixclock: 10000000 / framebuffer.width() as u32 * 1000
                            / framebuffer.height() as u32,
                        left_margin: (framebuffer.width() as u32 / 8) & 0xf8,
                        right_margin: 32,
                        upper_margin: 16,
                        lower_margin: 4,
                        vsync_len: 4,
                        hsync_len: (framebuffer.width() as u32 / 8) & 0xf8,
                        ..Default::default()
                    };

                    current_userspace!().write_val(arg, &screen_info)?;

                    Ok(0)
                } else {
                    println!("Framebuffer is not initialized");
                    return_errno!(Errno::ENODEV); // No such device
                }
            }
            IoctlCmd::GETFSCREENINFO => {
                println!("Fb ioctl: Get fixed screen info");

                // Use get_framebuffer_info to access the framebuffer
                if let Some(framebuffer_guard) = get_framebuffer_info() {
                    let framebuffer = &*framebuffer_guard;

                    // FIXME: On demand add more fields
                    let screen_info = FbFixScreenInfo {
                        smem_start: framebuffer.io_mem_base() as u64,
                        smem_len: (framebuffer.width()
                            * framebuffer.height()
                            * framebuffer.bytes_per_pixel())
                            as u32,
                        line_length: (framebuffer.width() * framebuffer.bytes_per_pixel()) as u32,
                        ..Default::default()
                    };

                    current_userspace!().write_val(arg, &screen_info)?;

                    Ok(0)
                } else {
                    println!("Framebuffer is not initialized");
                    return_errno!(Errno::ENODEV); // No such device
                }
            }
            IoctlCmd::PUTVSCREENINFO => {
                // Not support for efifb
                // Behavior is aligned with Linux
                Ok(0)
            }
            IoctlCmd::GETCMAP => {
                println!("Fb ioctl: Get color map");
                // Implement logic to get the color map
                Ok(0)
            }
            IoctlCmd::PUTCMAP => {
                println!("Fb ioctl: Set color map");
                // Implement logic to set the color map
                Ok(0)
            }
            IoctlCmd::PANDISPLAY => {
                // Not support for efifb
                // Behavior is aligned with Linux
                return_errno!(Errno::EINVAL);
            }
            IoctlCmd::FBIOBLANK => {
                // Not support for efifb
                // Behavior is aligned with Linux
                return_errno!(Errno::EINVAL);
            }
            _ => {
                println!("Fb ioctl: Unsupported command -> {:?}", cmd);
                return_errno!(Errno::EINVAL); // Invalid argument error
            }
        }
    }
}
