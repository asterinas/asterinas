// SPDX-License-Identifier: MPL-2.0

//! Device classes, their devices, and observers.
//!
//! A class is what a device looks like to user space, regardless of how it is attached:
//! a block device, a terminal, a memory device.
//! A [`Class`] implementation supplies the per-device payload type,
//! the `/dev` naming policy, and class-wide attributes.
//! Registering it yields a [`ClassHandle`], which owns `/sys/class/<name>` and the member list,
//! and lets [`ClassObserver`]s subscribe to membership changes.
//!
//! To define a class, implement [`Class`] and pass it to [`register`].
//! Create its devices with [`ClassDevice::builder`] using the returned handle,
//! set a device number when a `/dev` node is needed,
//! then register each device with [`add_device`].
//! [`Class::dev_attrs`] defines attributes shared by the class's devices,
//! and [`Class::devnode`] customizes their `/dev` paths and permissions.
//!
//! To track devices created by other code, implement [`ClassObserver`]
//! and register it through [`ClassHandle::register_observer`].
//! It is notified about existing members as well as later additions and removals.
//! Follow the callback restrictions documented on [`ClassObserver`].
//!
//! [`add_device`]: crate::add_device

mod device;
mod observer;

use alloc::{sync::Arc, vec::Vec};

use ostd::sync::Mutex;

pub use self::{
    device::{ClassDevice, ClassDeviceBuilder},
    observer::ClassObserver,
};
use crate::common::{
    AnyDevice, DevNode, DeviceInternals, Error, Result, SysStr,
    attr::Attr,
    node::{Dir, SysTreeEdit},
    registry,
    subsystem::SubsystemOps,
};

/// A kind of class.
pub trait Class: Sized + Send + Sync + 'static {
    /// The class name: the directory under `/sys/class`.
    ///
    /// Must be a valid node name according to [`aster_systree::is_valid_name`].
    const NAME: &'static str;

    /// The payload type of devices on this class.
    type Device: Send + Sync + 'static;

    /// Overrides the `/dev` node name or mode of a device.
    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        None
    }

    /// Returns the attributes shared by all devices on this class.
    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>];

    /// Whether a device of this class placed under a class device still gets its own glue directory.
    const KEEPS_GLUE_DIR: bool = false;
}

/// Registers a class, creating `/sys/class/<name>`.
///
/// # Panics
///
/// Panics if the class name is not a valid `SysTree` node name.
pub fn register<C: Class>(class: C) -> Result<Arc<ClassHandle<C>>> {
    let dir = Dir::new(SysStr::from(C::NAME));
    registry::get().class_root().attach_child(dir.clone())?;
    let handle = Arc::new(ClassHandle {
        class,
        dir,
        devices: Mutex::new(Vec::new()),
        observers: Mutex::new(Vec::new()),
        membership: Mutex::new(()),
    });
    registry::get().keep_subsystem(handle.clone());
    Ok(handle)
}

/// A registered class.
pub struct ClassHandle<C: Class> {
    class: C,
    dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<ClassDevice<C>>>>,
    observers: Mutex<Vec<Arc<dyn ClassObserver<C>>>>,
    /// Serializes membership changes with observer registration,
    /// so that an observer sees each member exactly once.
    /// Observer callbacks run under it; see `ClassObserver` for restrictions.
    membership: Mutex<()>,
}

impl<C: Class> ClassHandle<C> {
    /// Registers an observer and announces the current members
    /// through [`on_device_added`].
    ///
    /// The callback restrictions in [`ClassObserver`] apply to these notifications.
    ///
    /// [`on_device_added`]: ClassObserver::on_device_added
    pub fn register_observer(&self, observer: Arc<dyn ClassObserver<C>>) -> Result<()> {
        let _guard = self.membership.lock();
        {
            let mut observers = self.observers.lock();
            if observers.iter().any(|o| Arc::ptr_eq(o, &observer)) {
                return Err(Error::AlreadyAdded);
            }
            observers.push(observer.clone());
        }
        for dev in self.devices() {
            observer.on_device_added(&dev);
        }
        Ok(())
    }

    /// Unregisters an observer and announces the current members
    /// through [`on_device_removed`].
    ///
    /// The callback restrictions in [`ClassObserver`] apply to these notifications.
    ///
    /// [`on_device_removed`]: ClassObserver::on_device_removed
    pub fn unregister_observer(&self, observer: &Arc<dyn ClassObserver<C>>) -> Result<()> {
        let _guard = self.membership.lock();
        let removed = {
            let mut observers = self.observers.lock();
            let before = observers.len();
            observers.retain(|o| !Arc::ptr_eq(o, observer));
            observers.len() != before
        };
        if !removed {
            return Err(Error::NotFound);
        }
        for dev in self.devices() {
            observer.on_device_removed(&dev);
        }
        Ok(())
    }

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

    fn observers(&self) -> Vec<Arc<dyn ClassObserver<C>>> {
        self.observers.lock().clone()
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
        let dev = dev
            .as_any()
            .downcast_ref::<ClassDevice<C>>()
            .expect("a class device reports its own class as its subsystem");
        let dev = dev.this();
        let _guard = self.membership.lock();
        self.devices.lock().push(dev.clone());
        for observer in self.observers() {
            observer.on_device_added(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<ClassDevice<C>>()
            .expect("a class device reports its own class as its subsystem");
        let dev = dev.this();
        let _guard = self.membership.lock();
        // A device whose registration failed before `on_added` was never
        // announced to the observers, so it is not un-announced either.
        let is_member = self.devices.lock().iter().any(|d| Arc::ptr_eq(d, &dev));
        if !is_member {
            return;
        }
        // Notify observers before removing the device from the member list,
        // so they can still find it when enumerating the class.
        for observer in self.observers() {
            observer.on_device_removed(&dev);
        }
        self.devices.lock().retain(|d| !Arc::ptr_eq(d, &dev));
    }
}

impl<C: Class> core::fmt::Debug for ClassHandle<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassHandle")
            .field("name", &C::NAME)
            .finish()
    }
}
