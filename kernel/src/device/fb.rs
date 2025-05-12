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
#[derive(Debug, Clone, Copy, Pod)]
pub struct FbVarScreenInfo {
    pub xres: u32,
    pub yres: u32,
    pub xres_virtual: u32,
    pub yres_virtual: u32,
    pub bits_per_pixel: u32,
    pub red: FbBitfield,
    pub green: FbBitfield,
    pub blue: FbBitfield,
    pub transp: FbBitfield,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct FbBitfield {
    pub offset: u32,
    pub length: u32,
    pub msb_right: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct FbFixScreenInfo {
    pub smem_start: usize, // Start of framebuffer memory
    pub smem_len: usize,   // Length of framebuffer memory
    pub line_length: usize, // Length of a line in bytes
                           // Add other fields as needed
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
                        bits_per_pixel: (pixel_format.nbytes() * 8) as u32,
                        red: red_bitfield,
                        green: green_bitfield,
                        blue: blue_bitfield,
                        transp: transp_bitfield,
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

                    let screen_info = FbFixScreenInfo {
                        smem_start: framebuffer.io_mem_base(),
                        smem_len: framebuffer.width()
                            * framebuffer.height()
                            * framebuffer.bytes_per_pixel(),
                        line_length: framebuffer.width() * framebuffer.bytes_per_pixel(),
                    };

                    current_userspace!().write_val(arg, &screen_info)?;

                    Ok(0)
                } else {
                    println!("Framebuffer is not initialized");
                    return_errno!(Errno::ENODEV); // No such device
                }
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
                println!("Fb ioctl: Pan display");
                let offset = arg; // Assume `arg` contains the offset value
                println!("Panning display to offset: {}", offset);

                // Implement logic to pan the display
                Ok(0)
            }
            IoctlCmd::FBIOBLANK => {
                println!("Fb ioctl: Blank screen");
                let blank_mode = arg; // Assume `arg` contains the blank mode
                println!("Setting blank mode to: {}", blank_mode);

                // Implement logic to blank the screen
                Ok(0)
            }
            _ => {
                println!("Fb ioctl: Unsupported command -> {:?}", cmd);
                return_errno!(Errno::EINVAL); // Invalid argument error
            }
        }
    }
}
