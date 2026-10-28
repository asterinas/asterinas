// SPDX-License-Identifier: MPL-2.0

use core::fmt::Write;

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode, DevNum, Result as DeviceResult},
};
use device_id::{DeviceId, MajorId, MinorId};
use spin::Once;

use super::device::SystemConsole;
use crate::{
    device::{Device, DeviceType, registry::char},
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, mkmod},
    },
    prelude::*,
};

static TTY_MAJOR: Once<char::MajorIdOwner> = Once::new();
static TTY_CLASS: Once<Arc<ClassHandle<TtyClass>>> = Once::new();

/// The device ID of `/dev/console`.
pub(crate) fn console_device_id() -> DeviceId {
    let major = TTY_MAJOR
        .call_once(|| char::acquire_major(MajorId::new(5)).unwrap())
        .get();
    DeviceId::new(major, MinorId::new(TtyPayload::CONSOLE_MINOR))
}

pub(super) fn init_in_first_process() -> Result<()> {
    // A no-op if `console_device_id` has already acquired the major.
    TTY_MAJOR.call_once(|| char::acquire_major(MajorId::new(5)).unwrap());

    let handle = class::register(TtyClass)?;
    TTY_CLASS.call_once(move || handle);

    add_tty_device(TtyPayload::Console(SystemConsole::from_console_param()))
}

fn add_tty_device(payload: TtyPayload) -> Result<()> {
    let class = TTY_CLASS.get().unwrap();
    let name = payload.name();
    let devnum = payload.devnum();
    let attrs = payload.attrs();
    let device = TtyClassDevice::builder(class, name, payload)
        .devnum(devnum)
        .attrs(attrs)
        .build();

    aster_device::add_device(&device)?;
    if let Err(error) = char::register(device.clone()) {
        let _ = aster_device::remove_device(&device);
        return Err(error);
    }
    Ok(())
}

/// The `tty` class, which owns `/sys/class/tty`.
struct TtyClass;

impl Class for TtyClass {
    const NAME: &'static str = "tty";
    type Device = TtyPayload;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        let mode = match dev.payload() {
            TtyPayload::Console(_) => mkmod!(u+rw),
        };
        Some(DevNode {
            path: None,
            mode: Some(mode.bits()),
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

static CONSOLE_DEV_ATTRS: [Attr<ClassDevice<TtyClass>>; 1] = [Attr::ro("active", show_active)];

fn show_active(dev: &ClassDevice<TtyClass>, writer: &mut dyn Write) -> DeviceResult<()> {
    let name = match dev.payload() {
        TtyPayload::Console(console) => console.active_name(),
    };
    writeln!(writer, "{}", name)?;
    Ok(())
}

/// The payload of a device in the `tty` class.
enum TtyPayload {
    /// `/dev/console`, which forwards I/O to the TTY selected by the `console=` parameter.
    Console(SystemConsole),
}

impl TtyPayload {
    const CONSOLE_MINOR: u32 = 1;

    fn name(&self) -> &'static str {
        match self {
            TtyPayload::Console(_) => "console",
        }
    }

    fn minor(&self) -> u32 {
        match self {
            TtyPayload::Console(_) => Self::CONSOLE_MINOR,
        }
    }

    fn devnum(&self) -> DevNum {
        DevNum::char(DeviceId::new(
            TTY_MAJOR.get().unwrap().get(),
            MinorId::new(self.minor()),
        ))
    }

    fn attrs(&self) -> &'static [Attr<TtyClassDevice>] {
        match self {
            TtyPayload::Console(_) => &CONSOLE_DEV_ATTRS,
        }
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        match self {
            TtyPayload::Console(console) => console.open(),
        }
    }
}

/// A device in the `tty` class, as seen by the char device registry.
type TtyClassDevice = ClassDevice<TtyClass>;

impl Device for TtyClassDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.devnum()
            .expect("TTY devices always have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        // The device model creates the node when the device is added.
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        self.payload().open()
    }
}
