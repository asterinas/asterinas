// SPDX-License-Identifier: MPL-2.0

//! Classes.
//!
//! A class is what a device looks like to user space, regardless of how it
//! is attached: a block device, a terminal, a memory device. A [`Class`]
//! implementation supplies the per-device payload type, the `/dev` naming
//! policy, and class-wide attributes. Registering it yields a
//! [`ClassHandle`], which owns `/sys/class/<name>` and the member list, and
//! lets [`ClassInterface`]s subscribe to membership changes.

use alloc::{sync::Arc, vec::Vec};

use ostd::sync::Mutex;

use crate::{
    Error, Result, SysStr,
    attr::Attr,
    device::{AnyDevice, ClassDevice, DevNode, SubsystemOps},
    node::{Dir, SysTreeEdit},
    uevent::UeventVars,
};

/// A kind of class.
pub trait Class: Sized + Send + Sync + 'static {
    /// The class name: the directory under `/sys/class`.
    const NAME: &'static str;

    /// What every device in this class carries.
    type Device: Send + Sync + 'static;

    /// Overrides the `/dev` node name or mode of a device.
    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        None
    }

    /// Attributes every device in this class gets.
    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }

    /// Adds class-specific uevent variables.
    fn uevent(&self, _dev: &ClassDevice<Self>, _vars: &mut UeventVars) {}

    /// Whether a device of this class placed under a class device still gets
    /// its own glue directory. Linux does this for classes that define a
    /// sysfs namespace type, such as `net`, whose glue directory is what
    /// scopes the names to a namespace.
    const KEEPS_GLUE_DIR: bool = false;
}

/// Callbacks run for every device that joins or leaves a class.
///
/// This is for code that must act on *every* member of a class without being
/// the code that creates them. Without it, each such consumer has to be called
/// by hand from every producer, which is why each subsystem in Asterinas keeps
/// a device list of its own (`aster_block::DEVICE_REGISTRY`,
/// `aster_console::console_device_table`, `EVDEV_DEVICES`). An interface
/// inverts that: the class is the one place that knows its members, and a
/// consumer subscribes to it.
///
/// One subsystem has already built this privately: `aster-input`'s
/// `InputHandlerClass` is a class interface under another name, replayed in
/// both directions, and it is how evdev gets an `/dev/input/eventN` for every
/// input device without virtio-input calling it. The two that have not:
///
/// - **A layer built on top of a class.** When a disk joins the `block` class,
///   something has to read its partition table and register a device per
///   partition. That code belongs to the partition layer, not to virtio-blk or
///   NVMe, and it must run for disks from every driver.
/// - **A chooser.** The console picks among the members of the `tty` class by
///   walking a list; it would rather be told as they appear.
///
/// [`ClassHandle::register_interface`] replays `add_dev` for the members that
/// already exist, and [`ClassHandle::unregister_interface`] replays
/// `remove_dev` for those still present. That is what makes initialization
/// order free: the partition layer sees the same set of disks whether it
/// starts before or after the disk drivers, and it sees each disk exactly
/// once.
///
/// Linux's counterpart is `class_interface_register`, which the SCSI core uses
/// this way through `scsi_register_interface`, under the same per-class lock.
pub trait ClassInterface<C: Class>: Send + Sync + 'static {
    /// A device has joined the class.
    fn add_dev(&self, dev: &Arc<ClassDevice<C>>);

    /// A device is leaving the class.
    fn remove_dev(&self, _dev: &Arc<ClassDevice<C>>) {}
}

/// A registered class.
pub struct ClassHandle<C: Class> {
    class: C,
    dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<ClassDevice<C>>>>,
    interfaces: Mutex<Vec<Arc<dyn ClassInterface<C>>>>,
    /// Serializes membership changes with interface registration, so that an
    /// interface sees each member exactly once (Linux's `sp->mutex`).
    /// Interface callbacks run under it and must not add or remove devices
    /// of this class.
    membership: Mutex<()>,
}

impl<C: Class> core::fmt::Debug for ClassHandle<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassHandle")
            .field("name", &C::NAME)
            .finish()
    }
}

/// Registers a class, creating `/sys/class/<name>`.
pub fn register_class<C: Class>(class: C) -> Result<Arc<ClassHandle<C>>> {
    let dir = Dir::new(SysStr::from(C::NAME));
    crate::registry().class_root().attach_child(dir.clone())?;
    let handle = Arc::new(ClassHandle {
        class,
        dir,
        devices: Mutex::new(Vec::new()),
        interfaces: Mutex::new(Vec::new()),
        membership: Mutex::new(()),
    });
    crate::registry().keep_subsystem(handle.clone());
    Ok(handle)
}

impl<C: Class> ClassHandle<C> {
    /// Returns the class itself.
    pub fn class(&self) -> &C {
        &self.class
    }

    /// Returns the devices currently in the class.
    pub fn devices(&self) -> Vec<Arc<ClassDevice<C>>> {
        self.devices.lock().clone()
    }

    /// Finds a device in the class by name.
    pub fn find_device(&self, name: &str) -> Option<Arc<ClassDevice<C>>> {
        self.devices
            .lock()
            .iter()
            .find(|d| d.base().name() == name)
            .cloned()
    }

    /// Registers an interface and replays `add_dev` for the current members.
    ///
    /// `add_dev` runs under the class's membership lock and must not add or
    /// remove devices of this class.
    pub fn register_interface(&self, interface: Arc<dyn ClassInterface<C>>) {
        let _guard = self.membership.lock();
        self.interfaces.lock().push(interface.clone());
        for dev in self.devices() {
            interface.add_dev(&dev);
        }
    }

    /// Unregisters an interface, calling `remove_dev` for the current members.
    pub fn unregister_interface(&self, interface: &Arc<dyn ClassInterface<C>>) -> Result<()> {
        let _guard = self.membership.lock();
        let removed = {
            let mut interfaces = self.interfaces.lock();
            let before = interfaces.len();
            interfaces.retain(|i| !Arc::ptr_eq(i, interface));
            interfaces.len() != before
        };
        if !removed {
            return Err(Error::NotFound);
        }
        for dev in self.devices() {
            interface.remove_dev(&dev);
        }
        Ok(())
    }

    fn interfaces(&self) -> Vec<Arc<dyn ClassInterface<C>>> {
        self.interfaces.lock().clone()
    }
}

impl<C: Class> SubsystemOps for ClassHandle<C> {
    fn name(&self) -> &'static str {
        C::NAME
    }

    fn dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn index_dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn keeps_glue_dir(&self) -> bool {
        C::KEEPS_GLUE_DIR
    }

    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let Some(dev) = dev.as_any().downcast_ref::<ClassDevice<C>>() else {
            // Only a `ClassDevice<C>` reports this class as its subsystem.
            ostd::error!("a device of another type was added to class {}", C::NAME);
            return;
        };
        let dev = dev.this();
        let _guard = self.membership.lock();
        self.devices.lock().push(dev.clone());
        for interface in self.interfaces() {
            interface.add_dev(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let Some(dev) = dev.as_any().downcast_ref::<ClassDevice<C>>() else {
            ostd::error!(
                "a device of another type was removed from class {}",
                C::NAME
            );
            return;
        };
        let dev = dev.this();
        let _guard = self.membership.lock();
        // A device whose registration failed before `on_added` was never
        // announced to the interfaces, so it is not un-announced either.
        let is_member = self.devices.lock().iter().any(|d| Arc::ptr_eq(d, &dev));
        if !is_member {
            return;
        }
        // The interfaces are told first, so that one walking the class still
        // sees the departing member, as it does in Linux.
        for interface in self.interfaces() {
            interface.remove_dev(&dev);
        }
        self.devices.lock().retain(|d| !Arc::ptr_eq(d, &dev));
    }
}
