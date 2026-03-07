// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};

use aster_console::AnyConsoleDevice;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{Tty, TtyDriver};
use crate::{
    device::{
        registry::char,
        tty::{TtyFile, termio::CTermios},
    },
    fs::inode_handle::FileIo,
    prelude::*,
};

/// The driver for serial devices.
#[derive(Clone)]
pub struct SerialDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl SerialDriver {
    const MINOR_ID_BASE: u32 = 64;
}

impl TtyDriver for SerialDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn devtmpfs_path(&self, index: u32) -> Option<String> {
        Some(format!("ttyS{}", index - Self::MINOR_ID_BASE))
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(TtyFile(tty)))
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

static SERIAL0: Once<Arc<Tty<SerialDriver>>> = Once::new();

/// Returns the `ttyS0` device.
///
/// Returns `None` if the device is not found nor initialized.
pub fn serial0_device() -> Option<&'static Arc<Tty<SerialDriver>>> {
    SERIAL0.get()
}

pub(super) fn init_in_first_process() -> Result<()> {
    let devices = aster_console::all_devices();

    // Initialize the `ttyS0` device if the serial console is available.

    let serial_console = devices
        .iter()
        .find(|(name, _)| name.as_str() == aster_uart::CONSOLE_NAME)
        .map(|(_, device)| device.clone());

    if let Some(serial_console) = serial_console {
        let driver = SerialDriver {
            console: serial_console.clone(),
        };
        let serial0 = Tty::new(SerialDriver::MINOR_ID_BASE, driver);

        SERIAL0.call_once(|| serial0.clone());
        char::register(serial0.clone())?;

        serial_console.register_callback(Box::leak(Box::new(
            move |mut reader: VmReader<Infallible>| {
                let mut chs = vec![0u8; reader.remain()];
                reader.read(&mut VmWriter::from(chs.as_mut_slice()));
                let _ = serial0.push_input(chs.as_slice());
            },
        )));
    }

    Ok(())
}
