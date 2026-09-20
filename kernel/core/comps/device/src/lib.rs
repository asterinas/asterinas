// SPDX-License-Identifier: MPL-2.0

//! The device model: devices, buses, classes, and drivers, and the sysfs view built from them.
//!
//! Linux organizes every device it manages in one tree,
//! matches devices with drivers through buses,
//! presents them to user space through classes,
//! and shows all of it under `/sys`.
//! This crate is the Asterinas counterpart.
//! It sits between the hardware components (which enumerate devices and implement drivers)
//! and the `systree` component (which sysfs displays),
//! and it owns every node under `/sys/devices`, `/sys/bus`, `/sys/class`, and `/sys/dev`.
//!
//! The design follows Linux's behavior but not its shapes.
//! Where Linux has one `struct device` and one `struct kobject` that any code may edit,
//! this crate has:
//!
//! - typed devices, [`BusDevice<B>`] and [`ClassDevice<C>`],
//!   whose payloads drivers and classes see with their concrete types,
//!   plus [`BareDevice`] for parents that are neither;
//! - one erased view, [`AnyDevice`],
//!   that the registration sequence, the tree, and parent links use;
//! - [`Bus`] and [`Class`] as traits with associated payload types,
//!   so that a device belongs to exactly one of them by construction;
//! - a single registration function, [`add`], that performs every step device registration needs,
//!   and a single [`remove`].
//!
//! Device nodes are created through [`KernelHooks`] installed by the kernel crate,
//! where devtmpfs lives.
//! Node creation requests are queued until [`install_hooks`] installs the hooks and replays them.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "device: "
    };
}

mod attr;
mod bus;
mod class;
mod device;
mod devnum;
mod driver;
mod error;
mod hooks;
mod node;
mod subsystem;

use alloc::{sync::Arc, vec::Vec};

use aster_systree::{SysObj, primary_tree};
use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

pub use self::{
    attr::{Attr, ShowFn, StoreFn},
    bus::{Bus, BusHandle, register_bus},
    class::{Class, ClassHandle, ClassInterface, register_class},
    device::{
        AnyDevice, BareDevice, BusDevice, BusDeviceBuilder, ClassDevice, ClassDeviceBuilder,
        DevNode, DeviceBase, DeviceBuilder, DeviceType, add, remove,
    },
    devnum::{DEFAULT_DEVNODE_MODE, DevKind, DevNodeRequest, DevNum},
    driver::{Driver, DriverHandle},
    error::{Error, Result},
    hooks::{HookError, KernelHooks, install_hooks},
    node::Container,
    subsystem::{Subsystem, SubsystemKind},
};
use self::{
    node::{Dir, GlueDirs, SysTreeEdit},
    subsystem::SubsystemOps,
};

/// An owned string or a static reference to string.
pub type SysStr = aster_systree::SysStr;

#[init_component]
fn init() -> core::result::Result<(), ComponentInitError> {
    REGISTRY.call_once(|| Registry::new().expect("cannot create the device model roots"));
    Ok(())
}

fn registry() -> &'static Registry {
    REGISTRY.get().expect("the device model is not initialized")
}

static REGISTRY: Once<Registry> = Once::new();

/// The top-level directories the device model owns.
struct Registry {
    devices: Arc<Dir>,
    virtual_dir: Arc<Dir>,
    bus: Arc<Dir>,
    class: Arc<Dir>,
    dev_char: Arc<Dir>,
    dev_block: Arc<Dir>,
    virtual_glue_dirs: GlueDirs,
    /// The registered buses and classes, kept alive for the life of the kernel
    /// (Linux's `bus_kset` and `class_kset` own them the same way).
    subsystems: Mutex<Vec<Arc<dyn SubsystemOps>>>,
}

impl Registry {
    fn new() -> Result<Self> {
        let devices = Dir::new(SysStr::from("devices"));
        let virtual_dir = Dir::new(SysStr::from("virtual"));
        let bus = Dir::new(SysStr::from("bus"));
        let class = Dir::new(SysStr::from("class"));
        let dev = Dir::new(SysStr::from("dev"));
        let dev_char = Dir::new(SysStr::from("char"));
        let dev_block = Dir::new(SysStr::from("block"));

        devices.attach_child(virtual_dir.clone())?;
        dev.attach_child(dev_char.clone())?;
        dev.attach_child(dev_block.clone())?;
        let root = primary_tree().root();
        root.add_child(devices.clone())?;
        root.add_child(bus.clone())?;
        root.add_child(class.clone())?;
        root.add_child(dev)?;

        Ok(Self {
            devices,
            virtual_dir,
            bus,
            class,
            dev_char,
            dev_block,
            virtual_glue_dirs: GlueDirs::new(),
            subsystems: Mutex::new(Vec::new()),
        })
    }

    fn devices_root(&self) -> &Arc<Dir> {
        &self.devices
    }

    fn bus_root(&self) -> &Arc<Dir> {
        &self.bus
    }

    fn class_root(&self) -> &Arc<Dir> {
        &self.class
    }

    fn dev_index(&self, kind: DevKind) -> &Arc<Dir> {
        match kind {
            DevKind::Char => &self.dev_char,
            DevKind::Block => &self.dev_block,
        }
    }

    /// Keeps a registered bus or class alive for the life of the kernel.
    fn keep_subsystem(&self, subsystem: Arc<dyn SubsystemOps>) {
        self.subsystems.lock().push(subsystem);
    }

    /// Attaches `child` into `/sys/devices/virtual/<class>`,
    /// creating the directory if needed, and returns that directory.
    ///
    /// # Panics
    ///
    /// Panics if `class` is not a valid `SysTree` node name.
    fn attach_into_virtual_glue_dir(
        &self,
        class: &str,
        child: Arc<dyn SysObj>,
    ) -> Result<Arc<Dir>> {
        self.virtual_glue_dirs
            .attach_into(class, self.virtual_dir.as_ref(), child)
    }

    fn drop_virtual_glue_dir_if_empty(&self, class: &str) {
        self.virtual_glue_dirs
            .drop_if_empty(class, self.virtual_dir.as_ref());
    }
}
