// SPDX-License-Identifier: MPL-2.0

//! The framebuffer of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod ansi_escape;
mod console;
mod framebuffer;
mod pixel;

use component::{ComponentInitError, init_component};
pub use console::FramebufferConsole;
pub use framebuffer::{ColorMapEntry, FRAMEBUFFER, FrameBuffer, MAX_CMAP_SIZE};
pub use pixel::{Pixel, PixelFormat, RenderedPixel};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    framebuffer::init();
    Ok(())
}
