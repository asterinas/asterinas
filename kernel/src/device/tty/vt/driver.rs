// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};

use aster_console::AnyConsoleDevice;
use aster_framebuffer::DummyFramebufferConsole;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use crate::{
    device::{
        registry::char,
        tty::{Tty, TtyDriver, file::TtyFile, termio::CTermios},
    },
    fs::file::FileIo,
    prelude::*,
};

/// The driver for VT (virtual terminal) devices.
//
// TODO: This driver needs to support more features for future VT management.
#[derive(Clone)]
pub struct VtDriver {
    console: Arc<dyn AnyConsoleDevice>,
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

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        Some(&*self.console)
    }

    fn on_termios_change(&self, _old_termios: &CTermios, _new_termios: &CTermios) {}
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

pub fn init() -> Result<()> {
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
