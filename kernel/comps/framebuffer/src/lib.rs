// SPDX-License-Identifier: MPL-2.0

//! The framebuffer of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

mod ansi_escape;
mod console;
mod framebuffer;
mod pixel;

use component::{init_component, ComponentInitError};
pub use console::{FramebufferConsole, CONSOLE_NAME, FRAMEBUFFER_CONSOLE};
pub use framebuffer::{get_framebuffer_info, FrameBuffer, FRAMEBUFFER};
pub use pixel::{Pixel, PixelFormat, RenderedPixel};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    framebuffer::init();
    console::init();
    Ok(())
}
