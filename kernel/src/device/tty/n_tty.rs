// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc};

use aster_console::AnyConsoleDevice;
use aster_framebuffer::DummyFramebufferConsole;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{Tty, TtyDriver};
use crate::{fs::inode_handle::FileIo, prelude::*};

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

    fn open(tty: Arc<Tty<Self>>) -> Arc<dyn FileIo> {
        tty
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
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

    fn is_closed(&self) -> bool {
        false
    }

    fn notify_input(&self) {}

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        Some(&*self.console)
    }
}

/// The driver for hypervisor console devices.
#[derive(Clone)]
pub struct HvcDriver {
    console: Arc<dyn AnyConsoleDevice>,
}

impl TtyDriver for HvcDriver {
    // Reference: <https://github.com/torvalds/linux/blob/24172e0d79900908cf5ebf366600616d29c9b417/Documentation/admin-guide/devices.txt#L2936>.
    const DEVICE_MAJOR_ID: u32 = 229;

    fn open(tty: Arc<Tty<Self>>) -> Arc<dyn FileIo> {
        tty
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
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

    fn is_closed(&self) -> bool {
        false
    }

    fn notify_input(&self) {}

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        Some(&*self.console)
    }
}

static TTY1: Once<Arc<Tty<VtDriver>>> = Once::new();

static HVC0: Once<Arc<Tty<HvcDriver>>> = Once::new();

/// Returns the `TTY1` device.
///
/// # Panic
///
/// This function will panic if the `TTY1` device has not been initialized.
pub fn tty1_device() -> &'static Arc<Tty<VtDriver>> {
    TTY1.get().unwrap()
}

/// Returns the `HVC0` device.
///
/// Returns `None` if the device is not found nor initialized.
pub fn hvc0_device() -> Option<&'static Arc<Tty<HvcDriver>>> {
    HVC0.get()
}

pub(in crate::device) fn init_in_first_process() {
    let devices = aster_console::all_devices();

    // Initialize the `HVC0` device if virtio console is available.

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

        virtio_console.register_callback(Box::leak(Box::new(
            move |mut reader: VmReader<Infallible>| {
                let mut chs = vec![0u8; reader.remain()];
                reader.read(&mut VmWriter::from(chs.as_mut_slice()));
                let _ = hvc0.push_input(chs.as_slice());
            },
        )));
    }

    // Initialize the `tty1` device.

    let fb_device = devices
        .iter()
        .find(|(name, _)| name.as_str() == aster_framebuffer::CONSOLE_NAME)
        .map(|(_, device)| device.clone())
        .unwrap_or(Arc::new(DummyFramebufferConsole));

    let tty1 = {
        let driver = VtDriver {
            console: fb_device.clone(),
        };
        Tty::new(1, driver)
    };
    TTY1.call_once(|| tty1.clone());

    fb_device.register_callback(Box::leak(Box::new(
        move |mut reader: VmReader<Infallible>| {
            let mut chs = vec![0u8; reader.remain()];
            reader.read(&mut VmWriter::from(chs.as_mut_slice()));
            let _ = tty1.push_input(chs.as_slice());
        },
    )));
}
