// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format, sync::Arc};

use aster_console::AnyConsoleDevice;
use aster_framebuffer::DummyFramebufferConsole;
use inherit_methods_macro::inherit_methods;
use ostd::mm::{Infallible, VmReader, VmWriter};
use spin::Once;

use super::{Tty, TtyDriver};
use crate::{
    device::{registry::char, tty::termio::CTermios},
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
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

struct TtyFile<D>(Arc<Tty<D>>);

#[inherit_methods(from = "self.0")]
impl<D: TtyDriver> Pollable for TtyFile<D> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl<D: TtyDriver> InodeIo for TtyFile<D> {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.write(reader, status_flags)
    }
}

#[inherit_methods(from = "self.0")]
impl<D: TtyDriver> FileIo for TtyFile<D> {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}

static TTY1: Once<Arc<Tty<VtDriver>>> = Once::new();

static SERIAL0: Once<Arc<Tty<SerialDriver>>> = Once::new();

static HVC0: Once<Arc<Tty<HvcDriver>>> = Once::new();

/// Returns the `tty1` device.
///
/// # Panics
///
/// This function will panic if the `tty1` device has not been initialized.
pub fn tty1_device() -> &'static Arc<Tty<VtDriver>> {
    TTY1.get().unwrap()
}

/// Returns the `ttyS0` device.
///
/// Returns `None` if the device is not found nor initialized.
pub fn serial0_device() -> Option<&'static Arc<Tty<SerialDriver>>> {
    SERIAL0.get()
}

/// Returns the `hvc0` device.
///
/// Returns `None` if the device is not found nor initialized.
pub fn hvc0_device() -> Option<&'static Arc<Tty<HvcDriver>>> {
    HVC0.get()
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
