// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};

use aster_console::{
    AnyConsoleDevice,
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
};
use aster_framebuffer::DummyFramebufferConsole;
use ostd::mm::{Infallible, VmIo, VmReader, VmWriter};
use spin::Once;

use crate::{
    context::current_userspace,
    device::{
        registry::char,
        tty::{CFontOp, Tty, TtyDriver, file::TtyFile, termio::CTermios},
    },
    fs::file::FileIo,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The driver for VT (virtual terminal) devices.
//
// TODO: This driver needs to support more features for future VT management.
#[derive(Clone)]
pub struct VtDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl VtDriver {
    fn handle_set_font(&self, font_op: &CFontOp) -> Result<()> {
        let CFontOp {
            op,
            flags: _,
            width,
            height,
            charcount,
            data,
            ..
        } = font_op;

        let vpitch = match *op {
            CFontOp::OP_SET => CFontOp::NONTALL_VPITCH,
            CFontOp::OP_SET_TALL => font_op.height,
            CFontOp::OP_SET_DEFAULT => {
                return console_set_font(self.console.as_ref(), BitmapFont::new_basic8x8());
            }
            _ => return_errno_with_message!(Errno::EINVAL, "the font operation is invalid"),
        };

        if *width == 0
            || *height == 0
            || *width > CFontOp::MAX_WIDTH
            || *height > CFontOp::MAX_HEIGHT
            || *charcount > CFontOp::MAX_CHARCOUNT
            || *height > vpitch
        {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid or too large");
        }

        let font_size = width.div_ceil(u8::BITS) * vpitch * charcount;
        let mut font_data = vec![0; font_size as usize];
        current_userspace!().read_bytes(*data as Vaddr, &mut font_data[..])?;

        // In Linux, the most significant bit represents the first pixel, but `BitmapFont` requires
        // the least significant bit to represent the first pixel. So now we reverse the bits.
        font_data
            .iter_mut()
            .for_each(|byte| *byte = byte.reverse_bits());

        let font = BitmapFont::new_with_vpitch(
            *width as usize,
            *height as usize,
            vpitch as usize,
            font_data,
        );
        console_set_font(self.console.as_ref(), font)?;

        Ok(())
    }
}

fn console_set_font(console: &dyn AnyConsoleDevice, font: BitmapFont) -> Result<()> {
    use aster_console::ConsoleSetFontError;

    match console.set_font(font) {
        Ok(()) => Ok(()),
        Err(ConsoleSetFontError::InappropriateDevice) => {
            return_errno_with_message!(Errno::ENOTTY, "the console has no support for font setting")
        }
        Err(ConsoleSetFontError::InvalidFont) => {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid for the console")
        }
    }
}

impl TtyDriver for VtDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn devtmpfs_path(&self, index: u32) -> Option<String> {
        Some(format!("tty{}", index))
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(TtyFile::new(tty)))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        self.console.send(chs);
        Ok(chs.len())
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        |chs| self.console.send(chs)
    }

    fn can_push(&self) -> bool {
        true
    }

    fn notify_input(&self) {}

    fn on_termios_change(&self, _old_termios: &CTermios, _new_termios: &CTermios) {}

    fn ioctl(&self, _tty: &Tty<Self>, raw_ioctl: RawIoctl) -> Result<bool>
    where
        Self: Sized,
    {
        use super::ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ SetOrGetFont => {
                let font_op = cmd.read()?;
                self.handle_set_font(&font_op)?;
            }
            cmd @ SetGraphicsMode => {
                let mode = ConsoleMode::try_from(cmd.get())?;
                if !self.console.set_mode(mode) {
                    return_errno_with_message!(Errno::EINVAL, "the console mode is not supported");
                }
            }
            cmd @ GetGraphicsMode => {
                let mode = self.console.mode().unwrap_or(ConsoleMode::Text);
                cmd.write(&(mode as i32))?;
            }
            cmd @ SetKeyboardMode => {
                let mode = KeyboardMode::try_from(cmd.get())?;
                if !self.console.set_keyboard_mode(mode) {
                    return_errno_with_message!(Errno::EINVAL, "the keyboard mode is not supported");
                }
            }
            cmd @ GetKeyboardMode => {
                let mode = self.console.keyboard_mode().unwrap_or(KeyboardMode::Xlate);
                cmd.write(&(mode as i32))?;
            }
            _ => return Ok(false),
        });

        Ok(true)
    }
}

static TTY1: Once<Arc<Tty<VtDriver>>> = Once::new();

/// Returns the `tty1` device.
///
/// # Panics
///
/// This function will panic if the `tty1` device has not been initialized.
pub fn tty1_device() -> &'static Arc<Tty<VtDriver>> {
    TTY1.get().unwrap()
}

pub(super) fn init_in_first_process() -> Result<()> {
    let devices = aster_console::all_devices();

    // Initialize the `tty1` device.

    let fb_console = devices
        .iter()
        .find(|(name, _)| name.as_str() == aster_framebuffer::CONSOLE_NAME)
        .map(|(_, device)| device.clone())
        .unwrap_or_else(|| Arc::new(DummyFramebufferConsole));

    let driver = VtDriver {
        console: fb_console.clone(),
    };
    let tty1 = Tty::new(1, driver);

    TTY1.call_once(|| tty1.clone());
    char::register(tty1.clone())?;

    fb_console.register_callback(Box::leak(Box::new(
        move |mut reader: VmReader<Infallible>| {
            let mut chs = vec![0u8; reader.remain()];
            reader.read(&mut VmWriter::from(chs.as_mut_slice()));
            let _ = tty1.push_input(chs.as_slice());
        },
    )));

    Ok(())
}
