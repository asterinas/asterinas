// SPDX-License-Identifier: MPL-2.0

//! TTY devices.
//!
//! This module implements TTY devices such as `/dev/tty0`, `/dev/tty`, and `/dev/console`.
//!
//! Reference: <https://www.kernel.org/doc/html/latest/admin-guide/devices.html>

use device_id::{DeviceId, MajorId, MinorId};
use spin::Once;

use crate::{
    device::{
        Device, DeviceType, DevtmpfsInodeMeta,
        registry::char,
        tty::{hvc::hvc0_device, serial::serial0_device, vt::active_vt},
    },
    fs::file::{PerOpenFileOps, mkmod},
    prelude::*,
};

/// Corresponds to `/dev/tty0` in the file system. This device represents the active virtual
/// terminal.
pub struct Tty0Device;

impl Device for Tty0Device {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(4), MinorId::new(0))
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("tty0"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        active_vt().open()
    }
}

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of the current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(5), MinorId::new(0))
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        // Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/tty/tty_io.c#L3511>.
        Some(DevtmpfsInodeMeta::with_mode("tty", mkmod!(a+rw)))
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
pub struct SystemConsole {
    inner: Arc<dyn Device>,
}

impl SystemConsole {
    /// Returns the singleton instance of the console device.
    pub fn singleton() -> &'static Arc<SystemConsole> {
        static INSTANCE: Once<Arc<SystemConsole>> = Once::new();

        INSTANCE.call_once(|| {
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
            let inner = device.unwrap_or_else(|| {
                warn!(
                    "'{}' console not found, falling back to 'tty0'",
                    console_name
                );
                Arc::new(Tty0Device) as _
            });

            Arc::new(Self { inner })
        })
    }
}

impl Device for SystemConsole {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(5), MinorId::new(1))
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new("console"))
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        self.inner.open()
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    char::register(Arc::new(Tty0Device))?;
    char::register(Arc::new(TtyDevice))?;
    char::register(SystemConsole::singleton().clone())?;

    Ok(())
}

static CONSOLES: Once<Vec<String>> = Once::new();
aster_cmdline::define_repeatable_kv_param!("console", CONSOLES);
