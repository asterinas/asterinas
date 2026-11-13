// SPDX-License-Identifier: MPL-2.0

//! The `tty` device class.
//!
//! This module registers the `tty` class, which owns `/sys/class/tty`
//! and the `/dev` nodes of the TTY devices, and adds devices to the class.
//! Every TTY device in the class implements the [`AnyTtyDevice`] payload trait;
//! the char-registry view of such a device is a [`TtyClassDevice`].

use aster_device::{
    class::{Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode, DevNum, SysStr},
};
use device_id::DeviceId;
use spin::Once;

use super::device::SystemConsole;
use crate::{
    device::{Device, DeviceType},
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{InodeMode, PerOpenFileOps, mkmod},
    },
    prelude::*,
};

static TTY_CLASS: Once<Arc<ClassHandle<TtyClass>>> = Once::new();

/// Returns the device ID of `/dev/console`.
pub(crate) fn console_device_id() -> DeviceId {
    SystemConsole::DEVICE_ID
}

/// Registers the `tty` class, which owns `/sys/class/tty`.
///
/// This must be called before adding any device to the class.
pub(super) fn register() {
    let handle = aster_device::class::register(TtyClass).unwrap();
    TTY_CLASS.call_once(move || handle);
}

/// Adds a device to the `tty` class.
pub(super) fn add_tty_device(payload: Arc<dyn AnyTtyDevice>) -> Result<()> {
    let class = TTY_CLASS.get().unwrap();
    let name = payload.devnode_name();
    let devnum = DevNum::char(payload.device_id());
    let attrs = payload.attrs();
    let device = TtyClassDevice::builder(class, name, payload)
        .devnum(devnum)
        .attrs(attrs)
        .build();

    crate::device::add_class_char_device(&device)
}

/// Adds `/dev/console`, which forwards I/O to the TTY selected by the `console=` parameter.
///
/// This must be called after the other TTY devices are added.
pub(super) fn add_console_device() -> Result<()> {
    add_tty_device(Arc::new(SystemConsole::from_console_param()))
}

/// The `tty` class, which owns `/sys/class/tty`.
pub(super) struct TtyClass;

impl Class for TtyClass {
    const NAME: &'static str = "tty";
    type Device = Arc<dyn AnyTtyDevice>;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        Some(DevNode {
            path: None,
            mode: Some(dev.payload().devnode_mode().bits()),
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
}

/// The interface shared by the devices in the `tty` class.
pub(super) trait AnyTtyDevice: Send + Sync + 'static {
    /// Returns the device as `Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Returns the name of the `/sys/class/tty/<name>` entry and the `/dev` node.
    fn devnode_name(&self) -> SysStr;

    /// Returns the device number, shared by the class registration and the `/dev` node.
    fn device_id(&self) -> DeviceId;

    /// Returns the permission bits of the `/dev` node.
    fn devnode_mode(&self) -> InodeMode {
        mkmod!(u+rw)
    }

    /// Returns the attributes of this device under `/sys/class/tty/<name>/`.
    fn attrs(&self) -> &'static [Attr<TtyClassDevice>] {
        &[]
    }

    /// Opens the device.
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>>;
}

/// A device in the `tty` class, as seen by the char device registry.
pub(super) type TtyClassDevice = ClassDevice<TtyClass>;

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
