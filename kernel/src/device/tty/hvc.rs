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

/// The driver for hypervisor console devices.
#[derive(Clone)]
pub struct HvcDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl TtyDriver for HvcDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/Documentation/admin-guide/devices.txt#L2936>.
    const DEVICE_MAJOR_ID: u32 = 229;

    fn devtmpfs_path(&self, index: u32) -> Option<String> {
        Some(format!("hvc{}", index))
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

static HVC0: Once<Arc<Tty<HvcDriver>>> = Once::new();

/// Returns the `hvc0` device.
///
/// Returns `None` if the device is not found nor initialized.
pub fn hvc0_device() -> Option<&'static Arc<Tty<HvcDriver>>> {
    HVC0.get()
}

pub(super) fn init_in_first_process() -> Result<()> {
    let devices = aster_console::all_devices();

    // Initialize the `hvc0` device if the virtio console is available.

    let virtio_console = devices
        .iter()
        .find(|(name, _)| name.as_str() == aster_virtio::device::console::DEVICE_NAME)
        .map(|(_, device)| device.clone());

    if let Some(virtio_console) = virtio_console {
        let driver = HvcDriver {
            console: virtio_console.clone(),
        };
        let hvc0 = Tty::new(0, driver);

        HVC0.call_once(|| hvc0.clone());
        char::register(hvc0.clone())?;

        virtio_console.register_callback(Box::leak(Box::new(
            move |mut reader: VmReader<Infallible>| {
                let mut chs = vec![0u8; reader.remain()];
                reader.read(&mut VmWriter::from(chs.as_mut_slice()));
                let _ = hvc0.push_input(chs.as_slice());
            },
        )));
    }

    Ok(())
}
