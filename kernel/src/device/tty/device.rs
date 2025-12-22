// SPDX-License-Identifier: MPL-2.0

//! TTY devices.
//!
//! This module implements TTY devices such as `/dev/tty0`, `/dev/tty`, and `/dev/console`.
//!
//! Reference: <https://www.kernel.org/doc/html/latest/admin-guide/devices.html>

use aster_cmdline::KCMDLINE;
use device_id::{DeviceId, MajorId, MinorId};
use spin::Once;

use super::n_tty::tty1_device;
use crate::{
    device::{
        registry::char,
        tty::{
            Tty,
            n_tty::{VtDriver, hvc0_device, serial0_device},
        },
    },
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
    process::{JobControl, Terminal},
};

/// Corresponds to `/dev/tty0` in the file system. This device represents the active virtual
/// terminal.
pub struct Tty0Device;

impl Tty0Device {
    fn active_vt(&self) -> &Arc<Tty<VtDriver>> {
        // Currently there is only one virtual terminal `tty1`.
        tty1_device()
    }
}

impl Device for Tty0Device {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(4), MinorId::new(0))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("tty0".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        self.active_vt().open()
    }
}

impl Terminal for Tty0Device {
    fn job_control(&self) -> &JobControl {
        self.active_vt().job_control()
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

    fn devtmpfs_path(&self) -> Option<String> {
        Some("tty".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
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
    inner: Arc<dyn Terminal>,
}

impl SystemConsole {
    /// Returns the singleton instance of the console device.
    pub fn singleton() -> &'static Arc<SystemConsole> {
        static INSTANCE: Once<Arc<SystemConsole>> = Once::new();

        INSTANCE.call_once(|| {
            // TODO: Support specifying multiple TTY devices, e.g., "console=hvc0 console=tty0".
            let console_name = KCMDLINE
                .get()
                .unwrap()
                .get_console_names()
                .first()
                .map(String::as_str)
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

    /// Returns the terminal associated with the console device.
    pub fn terminal(&self) -> &Arc<dyn Terminal> {
        &self.inner
    }
}

impl Device for SystemConsole {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(5), MinorId::new(1))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        Some("console".into())
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        self.inner.open()
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    char::register(Arc::new(Tty0Device))?;
    char::register(Arc::new(TtyDevice))?;
    char::register(SystemConsole::singleton().clone())?;

    Ok(())
}
