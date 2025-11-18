// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;
use ostd::boot::boot_info;
use spin::Once;

use crate::{
    device::tty::{hvc0_device, n_tty::VtDriver, tty1_device, Tty},
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::StatusFlags,
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        JobControl, Terminal,
    },
};

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        let Some(terminal) = current!().terminal() else {
            return Some(Err(Error::with_message(
                Errno::ENOTTY,
                "the process does not have a controlling terminal",
            )));
        };

        terminal.open()
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(5, 0)
    }
}

impl Pollable for TtyDevice {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileIo for TtyDevice {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty device");
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty device");
    }
}

/// Corresponds to `/dev/tty0` in the file system. This device represents the active virtual
/// terminal.
pub struct Tty0Device;

impl Tty0Device {
    fn active_vt(&self) -> &Arc<Tty<VtDriver>> {
        // Currently there is only one virtual terminal tty1.
        tty1_device()
    }
}

impl Device for Tty0Device {
    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        Some(Ok(self.active_vt().clone() as Arc<dyn FileIo>))
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(4, 0)
    }
}

impl Terminal for Tty0Device {
    fn job_control(&self) -> &JobControl {
        self.active_vt().job_control()
    }
}

impl Pollable for Tty0Device {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileIo for Tty0Device {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty0 device");
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty0 device");
    }
}

/// Corresponds to `/dev/console` in the file system.
pub struct DevConsole {
    // TODO: Support specifying multiple tty devices.
    inner: Arc<dyn Terminal>,
}

impl DevConsole {
    /// Returns the singleton instance of the console device.
    pub fn singleton() -> &'static Arc<DevConsole> {
        static INSTANCE: Once<Arc<DevConsole>> = Once::new();
        INSTANCE.call_once(|| {
            let console_name = boot_info()
                .kernel_cmdline
                .split_whitespace()
                .find(|item| item.starts_with("console="))
                .and_then(|value| value.split('=').nth(1))
                .unwrap_or("tty0");

            let inner = match console_name {
                "tty0" => Arc::new(Tty0Device) as _,
                "hvc0" => {
                    if let Some(device) = hvc0_device() {
                        device.clone() as _
                    } else {
                        warn!("hvc0 device not found, falling back to 'tty0' console device");
                        Arc::new(Tty0Device) as _
                    }
                }
                _ => {
                    warn!(
                        "unsupported console device '{}', falling back to 'tty0'",
                        console_name
                    );
                    Arc::new(Tty0Device) as _
                }
            };

            Arc::new(Self { inner })
        })
    }

    /// Returns the terminal associated with the console device.
    pub fn terminal(&self) -> &Arc<dyn Terminal> {
        &self.inner
    }
}

impl Device for DevConsole {
    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        self.inner.open()
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(5, 1)
    }
}

impl Pollable for DevConsole {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileIo for DevConsole {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read console device");
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write console device");
    }
}
