// SPDX-License-Identifier: MPL-2.0

//! Buses, their devices, and drivers.
//!
//! A bus is the mechanism by which devices are found and addressed,
//! and it decides which driver serves which device.
//! A [`Bus`] implementation supplies the per-device payload type,
//! the match data drivers declare, and the matching rule.
//! Registering it yields a [`BusHandle`],
//! which owns the `/sys/bus/<name>` directory and the lists of devices and drivers.
//!
//! To add a new bus, implement [`Bus`] and pass it to [`register`].
//! For each discovered device, use [`BusDevice::builder`] with the returned handle,
//! then register the device with [`add_device`].
//!
//! To write a driver, implement [`Driver<B>`] for the target bus
//! and register it through [`BusHandle::register_driver`].
//! [`Driver::on_probe`] initializes a matched device, and
//! [`Driver::on_release`] releases its resources.
//! Driver attributes are declared with [`Attr`] through [`Driver::dev_attrs`].
//!
//! Binding is symmetric: a new device is offered to every registered driver,
//! and a new driver to every unbound device.
//! The first driver whose [`on_probe`] succeeds wins.
//! Automatic probing can be disabled through `/sys/bus/<name>/drivers_autoprobe`.
//!
//! [`add_device`]: crate::add_device
//! [`on_probe`]: Driver::on_probe

mod device;
mod driver;

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_systree::{SysObj, SysPerms};
use ostd::sync::Mutex;

pub use self::{
    device::{BusDevice, BusDeviceBuilder},
    driver::{Driver, DriverHandle},
};
use crate::common::{
    AnyDevice, DeviceInternals, Error, Result, SysStr,
    attr::Attr,
    node::{Dir, DirAttrOps, SysTreeEdit, add_link, remove_link},
    registry,
    subsystem::SubsystemOps,
};

/// A kind of bus.
pub trait Bus: Sized + Send + Sync + 'static {
    /// The bus name: the directory under `/sys/bus`.
    ///
    /// Must be a valid node name according to [`aster_systree::is_valid_name`].
    const NAME: &'static str;

    /// The payload type of devices on this bus.
    type Device: Send + Sync + 'static;

    /// What a driver declares to say which devices it accepts.
    type MatchData: Send + Sync + 'static;

    /// Decides whether a driver's match data accepts a device.
    fn matches(&self, dev: &Self::Device, data: &Self::MatchData) -> bool;

    /// Returns the attributes shared by all devices on this bus.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>];
}

/// Registers a bus, creating `/sys/bus/<name>/{devices,drivers}`.
///
/// # Panics
///
/// Panics if the bus name is not a valid `SysTree` node name.
pub fn register<B: Bus>(bus: B) -> Result<Arc<BusHandle<B>>> {
    let dir = Dir::with_attrs(
        SysStr::from(B::NAME),
        &[
            ("drivers_autoprobe", SysPerms::DEFAULT_RW_ATTR_PERMS),
            ("drivers_probe", SysPerms::OWNER_W),
        ],
    )?;
    let devices_dir = Dir::new(SysStr::from("devices"));
    let drivers_dir = Dir::new(SysStr::from("drivers"));
    dir.attach_child(devices_dir.clone())?;
    dir.attach_child(drivers_dir.clone())?;

    let handle = Arc::new_cyclic(|weak| BusHandle {
        bus,
        dir: dir.clone(),
        devices_dir,
        drivers_dir,
        devices: Mutex::new(Vec::new()),
        drivers: Mutex::new(Vec::new()),
        autoprobe: AtomicBool::new(true),
        weak: weak.clone(),
    });
    dir.set_ops(Box::new(BusDirOps {
        bus: handle.weak.clone(),
    }));
    registry::get().bus_root().attach_child(dir)?;
    registry::get().keep_subsystem(handle.clone());
    Ok(handle)
}

/// A registered bus.
pub struct BusHandle<B: Bus> {
    bus: B,
    dir: Arc<Dir>,
    devices_dir: Arc<Dir>,
    drivers_dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<BusDevice<B>>>>,
    drivers: Mutex<Vec<Arc<DriverHandle<B>>>>,
    autoprobe: AtomicBool,
    weak: Weak<Self>,
}

impl<B: Bus> BusHandle<B> {
    /// Registers a driver, creating `/sys/bus/<bus>/drivers/<name>`,
    /// and offers it every unbound device.
    ///
    /// # Panics
    ///
    /// Panics if the driver name is not a valid `SysTree` node name.
    pub fn register_driver(&self, driver: Arc<dyn Driver<B>>) -> Result<Arc<DriverHandle<B>>> {
        if self
            .drivers
            .lock()
            .iter()
            .any(|d| d.name() == driver.name())
        {
            return Err(Error::NameConflict);
        }
        let handle = DriverHandle::new(driver, self.weak.clone())?;
        self.drivers_dir.attach_child(handle.dir().clone())?;
        self.drivers.lock().push(handle.clone());

        if self.autoprobe.load(Ordering::Relaxed) {
            for dev in self.devices() {
                if dev.driver().is_none() {
                    let _ = self.try_bind(&dev, &handle);
                }
            }
        }
        Ok(handle)
    }

    /// Unregisters a driver, unbinding its devices first.
    ///
    /// Waits for ongoing probes and their cleanup before returning.
    pub fn unregister_driver(&self, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        let removed = {
            let mut drivers = self.drivers.lock();
            let before = drivers.len();
            drivers.retain(|d| !Arc::ptr_eq(d, driver));
            drivers.len() != before
        };
        if !removed {
            return Err(Error::NotFound);
        }
        // Close registration and collect every binding in one critical section.
        // Release this lock before waiting on each device's binding lock.
        let devices = driver.state().lock().set_unregistered()?;
        for dev in devices {
            let _ = self.unbind_from(&dev, driver);
        }
        let _ = self.drivers_dir.detach_child(driver.name());
        Ok(())
    }

    /// Unbinds the device from its driver.
    ///
    /// Removes the driver's attributes and links, calls its [`on_release`]
    /// callback, and clears the binding.
    ///
    /// Use this before [`remove_device`]
    /// when this device has child devices created by the driver.
    ///
    /// [`remove_device`]: crate::remove_device
    /// [`on_release`]: Driver::on_release
    pub fn unbind(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        self.unbind_inner(dev, None)
    }

    /// Binds a specific driver to a device, if the bus matches them.
    pub(crate) fn bind(
        &self,
        dev: &Arc<BusDevice<B>>,
        driver: &Arc<DriverHandle<B>>,
    ) -> Result<()> {
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        self.try_bind(dev, driver)
    }

    /// Offers an unbound device to every driver until one accepts it.
    fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        for driver in self.drivers() {
            if self.try_bind(dev, &driver).is_ok() {
                return Ok(());
            }
        }
        Err(Error::NoDriver)
    }

    /// Unbinds the device only if it is still bound to `expected`.
    ///
    /// Returns [`Error::NotBound`] if the device is unbound or has been bound to a different driver.
    fn unbind_from(&self, dev: &Arc<BusDevice<B>>, expected: &Arc<DriverHandle<B>>) -> Result<()> {
        self.unbind_inner(dev, Some(expected))
    }

    fn try_bind(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        if !self.bus.matches(dev.payload(), driver.match_data()) {
            return Err(Error::NoDriver);
        }
        let _guard = dev.bind_lock().lock();
        if !dev.base().is_added() {
            return Err(Error::NotAdded);
        }
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        driver.state().lock().start_binding(dev)?;

        let result = self.bind_inner(dev, driver);
        let mut state = driver.state().lock();
        if result.is_ok() {
            state.finish_binding(dev);
        } else {
            state.abort_binding(dev);
        }
        result
    }

    /// Completes a binding recorded in the driver's list, undoing it on error.
    /// The caller must hold the device's binding lock throughout.
    fn bind_inner(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        // Create the links before `on_probe` so it can find the device and driver through sysfs.
        // Remove them if probing fails.
        add_link(
            driver.dir().as_ref(),
            dev.base().name(),
            &dev.base().tree_path(),
        )?;
        if let Err(e) = add_link(dev.base(), "driver", &SysObj::path(driver.dir().as_ref())) {
            remove_link(driver.dir().as_ref(), dev.base().name());
            return Err(e);
        }
        if let Err(e) = driver.on_probe(dev) {
            driver::remove_driver_links(dev, driver);
            return Err(match e {
                Error::NoDriver => Error::NoDriver,
                _ => Error::ProbeFailed,
            });
        }
        if let Err(e) = dev.add_driver_attrs(driver) {
            driver::remove_driver_links(dev, driver);
            driver.on_release(dev);
            return Err(e);
        }
        dev.set_driver(driver.clone());
        Ok(())
    }

    fn unbind_inner(
        &self,
        dev: &Arc<BusDevice<B>>,
        expected: Option<&Arc<DriverHandle<B>>>,
    ) -> Result<()> {
        let _guard = dev.bind_lock().lock();
        if let Some(expected) = expected
            && !dev.driver().is_some_and(|d| Arc::ptr_eq(&d, expected))
        {
            return Err(Error::NotBound);
        }
        let driver = dev.driver().ok_or(Error::NotBound)?;
        // Driver attribute callbacks have finished and cannot start
        // while we hold the binding lock.
        driver::remove_driver_links(dev, &driver);
        dev.remove_attrs(driver.dev_attrs());
        driver.on_release(dev);
        dev.take_driver();
        driver.state().lock().forget_bound(dev);
        Ok(())
    }

    /// Returns whether new devices and drivers are matched automatically.
    fn is_autoprobe_enabled(&self) -> bool {
        self.autoprobe.load(Ordering::Relaxed)
    }

    /// Enables or disables automatic matching.
    fn set_autoprobe(&self, on: bool) {
        self.autoprobe.store(on, Ordering::Relaxed);
    }

    /// Returns the bus itself.
    fn bus(&self) -> &B {
        &self.bus
    }

    /// Returns the devices currently on the bus.
    fn devices(&self) -> Vec<Arc<BusDevice<B>>> {
        self.devices.lock().clone()
    }

    /// Returns the registered drivers.
    fn drivers(&self) -> Vec<Arc<DriverHandle<B>>> {
        self.drivers.lock().clone()
    }

    /// Finds a device on the bus by name.
    fn find_device(&self, name: &str) -> Option<Arc<BusDevice<B>>> {
        self.devices
            .lock()
            .iter()
            .find(|d| d.base().name() == name)
            .cloned()
    }
}

impl<B: Bus> SubsystemOps for BusHandle<B> {
    fn name(&self) -> &'static str {
        B::NAME
    }

    fn dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn index_dir(&self) -> Arc<Dir> {
        self.devices_dir.clone()
    }

    fn keeps_glue_dir(&self) -> bool {
        false
    }

    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<BusDevice<B>>()
            .expect("a bus device reports its own bus as its subsystem");
        let dev = dev.this();
        self.devices.lock().push(dev.clone());
        if self.autoprobe.load(Ordering::Relaxed) {
            // Automatic probing failure does not fail device registration.
            let _ = self.probe(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<BusDevice<B>>()
            .expect("a bus device reports its own bus as its subsystem");
        let dev = dev.this();
        let was_present = {
            let mut devices = self.devices.lock();
            let before = devices.len();
            devices.retain(|d| !Arc::ptr_eq(d, &dev));
            devices.len() != before
        };
        if was_present {
            let _ = self.unbind(&dev);
        }
    }
}

impl<B: Bus> core::fmt::Debug for BusHandle<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BusHandle").field("name", &B::NAME).finish()
    }
}

/// The `drivers_autoprobe` and `drivers_probe` files of a bus directory.
struct BusDirOps<B: Bus> {
    bus: Weak<BusHandle<B>>,
}

impl<B: Bus> DirAttrOps for BusDirOps<B> {
    fn show(&self, name: &str, w: &mut dyn Write) -> Result<()> {
        let bus = self.bus.upgrade().ok_or(Error::NotFound)?;
        match name {
            "drivers_autoprobe" => {
                writeln!(w, "{}", if bus.is_autoprobe_enabled() { 1 } else { 0 })?;
                Ok(())
            }
            _ => Err(Error::NotFound),
        }
    }

    fn store(&self, name: &str, value: &str) -> Result<()> {
        let bus = self.bus.upgrade().ok_or(Error::NotFound)?;
        match name {
            "drivers_autoprobe" => match value.trim() {
                "0" => bus.set_autoprobe(false),
                "1" => bus.set_autoprobe(true),
                _ => return Err(Error::InvalidValue),
            },
            "drivers_probe" => {
                let dev = bus.find_device(value.trim()).ok_or(Error::NotFound)?;
                // A probe request succeeds even if the device is already bound
                // or no driver matches.
                match bus.probe(&dev) {
                    Ok(()) | Err(Error::AlreadyBound) | Err(Error::NoDriver) => {}
                    Err(e) => return Err(e),
                }
            }
            _ => return Err(Error::NotFound),
        }
        Ok(())
    }
}
