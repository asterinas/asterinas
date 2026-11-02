// SPDX-License-Identifier: MPL-2.0

//! TTY devices.
//!
//! This module implements TTY devices such as `/dev/tty0`, `/dev/tty`, and `/dev/console`.
//!
//! Reference: <https://www.kernel.org/doc/html/latest/admin-guide/devices.html>

use aster_device::common::SysStr;
use device_id::{DeviceId, MajorId, MinorId};
use spin::Once;

use super::class::{self, AnyTtyDevice};
use crate::{
    device::tty::{hvc::hvc0_device, serial::serial0_device, vt::active_vt},
    fs::file::{InodeMode, PerOpenFileOps, mkmod},
    prelude::*,
};

/// Corresponds to `/dev/tty0` in the file system. This device represents the active virtual
/// terminal.
pub(crate) struct Tty0Device;

impl AnyTtyDevice for Tty0Device {
    fn devnode_name(&self) -> SysStr {
        SysStr::from("tty0")
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(4), MinorId::new(0))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        active_vt().open()
    }
}

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of the current process.
pub(crate) struct TtyDevice;

impl AnyTtyDevice for TtyDevice {
    fn devnode_name(&self) -> SysStr {
        SysStr::from("tty")
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(5), MinorId::new(0))
    }

    fn devnode_mode(&self) -> InodeMode {
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/tty/tty_io.c#L3511>.
        mkmod!(a+rw)
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        let Some(terminal) = current!().terminal() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the process does not have a controlling terminal"
            );
        };

        terminal.open()
    }
}

/// Corresponds to `/dev/console` in the file system. This device represents a console to which
/// system messages will be sent.
pub(super) struct SystemConsole {
    inner: Arc<dyn AnyTtyDevice>,
}

impl SystemConsole {
    /// The device ID of `/dev/console`.
    pub(crate) const DEVICE_ID: DeviceId = DeviceId::new(MajorId::new(5), MinorId::new(1));

    /// Creates the console device that forwards I/O to the TTY.
    pub(super) fn from_console_param() -> Self {
        // TODO: Support specifying multiple TTY devices, e.g., "console=hvc0 console=tty0".
        let console_name = CONSOLES
            .get()
            .and_then(|consoles| consoles.first().map(|s| s.as_str()))
            .unwrap_or("tty0");

        let device = match console_name {
            "tty0" => Some(Arc::new(Tty0Device) as _),
            "ttyS0" => serial0_device().cloned().map(|device| device as _),
            "hvc0" => hvc0_device().cloned().map(|device| device as _),
            _ => None,
        };
        let inner = match device {
            Some(inner) => inner,
            None => {
                warn!(
                    "'{}' console not found, falling back to 'tty0'",
                    console_name
                );
                Arc::new(Tty0Device) as _
            }
        };

        Self { inner }
    }
}

impl AnyTtyDevice for SystemConsole {
    fn devnode_name(&self) -> SysStr {
        SysStr::from("console")
    }

    fn device_id(&self) -> DeviceId {
        Self::DEVICE_ID
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        self.inner.open()
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    class::add_tty_device(Arc::new(Tty0Device))?;
    class::add_tty_device(Arc::new(TtyDevice))?;

    Ok(())
}

static CONSOLES: Once<Vec<String>> = Once::new();
aster_cmdline::define_repeatable_kv_param!("console", CONSOLES);
