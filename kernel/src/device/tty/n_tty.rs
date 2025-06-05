// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec};

use aster_console::AnyConsoleDevice;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{PushCharError, Tty, TtyDriver};
use crate::{
    error::Errno,
    prelude::{return_errno_with_message, Result},
};

pub struct ConsoleDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl TtyDriver for ConsoleDriver {
    fn push_output(&self, chs: &[u8]) -> core::result::Result<usize, PushCharError> {
        self.console.send(chs);
        Ok(chs.len())
    }

    fn drain_output(&self) {}

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        |chs| self.console.send(chs)
    }

    fn can_push(&self) -> bool {
        true
    }

    fn notify_input(&self) {}

    fn set_font(&self, font: aster_console::BitmapFont) -> Result<()> {
        use aster_console::ConsoleSetFontError;

        match self.console.set_font(font) {
            Ok(()) => Ok(()),
            Err(ConsoleSetFontError::InappropriateDevice) => {
                return_errno_with_message!(
                    Errno::ENOTTY,
                    "the console has no support for font setting"
                )
            }
            Err(ConsoleSetFontError::InvalidFont) => {
                return_errno_with_message!(Errno::EINVAL, "the font is invalid for the console")
            }
        }
    }
}

static N_TTY: Once<Box<[Arc<Tty<ConsoleDriver>>]>> = Once::new();

pub(in crate::device) fn init() {
    let devices = {
        let mut devices = aster_console::all_devices();
        // Sort by priorities to ensure that the TTY for the virtio-console device comes first. Is
        // there a better way than hardcoding this?
        devices.sort_by_key(|(name, _)| match name.as_str() {
            aster_virtio::device::console::DEVICE_NAME => 0,
            aster_framebuffer::CONSOLE_NAME => 1,
            _ => 2,
        });
        devices
    };

    let ttys = devices
        .into_iter()
        .enumerate()
        .map(|(index, (_, device))| create_n_tty(index as _, device))
        .collect();
    N_TTY.call_once(|| ttys);
}

fn create_n_tty(index: u32, device: Arc<dyn AnyConsoleDevice>) -> Arc<Tty<ConsoleDriver>> {
    let driver = ConsoleDriver {
        console: device.clone(),
    };

    let tty = Tty::new(index, driver);
    let tty_cloned = tty.clone();

    device.register_callback(Box::leak(Box::new(
        move |mut reader: VmReader<Infallible>| {
            let mut chs = vec![0u8; reader.remain()];
            reader.read(&mut VmWriter::from(chs.as_mut_slice()));
            let _ = tty.push_input(chs.as_slice());
        },
    )));

    tty_cloned
}

/// Returns the system console, i.e., `/dev/console`.
pub fn system_console() -> &'static Arc<Tty<ConsoleDriver>> {
    &N_TTY.get().unwrap()[0]
}

/// Iterates all TTY devices, i.e., `/dev/tty1`, `/dev/tty2`, e.t.c.
pub fn iter_n_tty() -> impl Iterator<Item = &'static Arc<Tty<ConsoleDriver>>> {
    N_TTY.get().unwrap().iter()
}
