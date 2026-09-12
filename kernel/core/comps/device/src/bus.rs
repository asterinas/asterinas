// SPDX-License-Identifier: MPL-2.0

//! Buses and drivers.
//!
//! A bus is the mechanism by which devices are found and addressed, and it
//! decides which driver serves which device. A [`Bus`] implementation
//! supplies the per-device payload type, the match data drivers declare, and
//! the matching rule. Registering it yields a [`BusHandle`], which owns the
//! `/sys/bus/<name>` directory and the lists of devices and drivers.
//!
//! Binding is symmetric: a new device is offered to every registered driver,
//! and a new driver to every unbound device. The first driver whose `probe`
//! succeeds wins.

use alloc::{
    string::ToString,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt::Write,
    ops::Deref,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_systree::{SysObj, SysPerms};
use ostd::sync::Mutex;

use crate::{
    Error, Result, SysStr,
    attr::Attr,
    device::{AnyDevice, BusDevice, SubsystemOps},
    node::{Dir, DirAttrOps, SysTreeEdit, add_link, remove_link},
    uevent::UeventVars,
};

/// A kind of bus.
pub trait Bus: Sized + Send + Sync + 'static {
    /// The bus name: the directory under `/sys/bus`.
    const NAME: &'static str;

    /// What every device on this bus carries: its address, identifiers,
    /// and resources.
    type Device: Send + Sync + 'static;

    /// What a driver declares to say which devices it accepts.
    type MatchData: Send + Sync + 'static;

    /// Decides whether a driver's match data accepts a device.
    fn matches(&self, dev: &Self::Device, data: &Self::MatchData) -> bool;

    /// Attributes every device on this bus gets.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        &[]
    }

    /// Adds bus-specific uevent variables, typically `MODALIAS`.
    fn uevent(&self, _dev: &BusDevice<Self>, _vars: &mut UeventVars) {}
}

/// A driver for devices on bus `B`.
pub trait Driver<B: Bus>: Send + Sync + 'static {
    /// The driver name: the directory under `/sys/bus/<bus>/drivers`.
    fn name(&self) -> &str;

    /// The devices this driver accepts, in the bus's terms.
    fn match_data(&self) -> &B::MatchData;

    /// Takes over a matched device. Return an error to decline it.
    fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()>;

    /// Releases a device before it is unbound or removed.
    fn remove(&self, _dev: &Arc<BusDevice<B>>) {}

    /// Attributes every bound device gets while it is bound.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<B>>] {
        &[]
    }
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

impl<B: Bus> core::fmt::Debug for BusHandle<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BusHandle").field("name", &B::NAME).finish()
    }
}

/// Registers a bus, creating `/sys/bus/<name>/{devices,drivers}`.
pub fn register_bus<B: Bus>(bus: B) -> Result<Arc<BusHandle<B>>> {
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
    dir.set_ops(Arc::new(BusDirOps {
        bus: handle.weak.clone(),
    }));
    crate::registry().bus_root().attach_child(dir)?;
    crate::registry().keep_subsystem(handle.clone());
    Ok(handle)
}

impl<B: Bus> BusHandle<B> {
    /// Returns the bus itself.
    pub fn bus(&self) -> &B {
        &self.bus
    }

    /// Returns the devices currently on the bus.
    pub fn devices(&self) -> Vec<Arc<BusDevice<B>>> {
        self.devices.lock().clone()
    }

    /// Returns the registered drivers.
    pub fn drivers(&self) -> Vec<Arc<DriverHandle<B>>> {
        self.drivers.lock().clone()
    }

    /// Finds a device on the bus by name.
    pub fn find_device(&self, name: &str) -> Option<Arc<BusDevice<B>>> {
        self.devices
            .lock()
            .iter()
            .find(|d| d.base().name() == name)
            .cloned()
    }

    /// Registers a driver, creating `/sys/bus/<bus>/drivers/<name>`, and
    /// offers it every unbound device.
    pub fn register_driver(&self, driver: Arc<dyn Driver<B>>) -> Result<Arc<DriverHandle<B>>> {
        if self
            .drivers
            .lock()
            .iter()
            .any(|d| d.name() == driver.name())
        {
            return Err(Error::NameConflict);
        }
        let dir = Dir::with_attrs(
            SysStr::from(driver.name().to_string()),
            &[("bind", SysPerms::OWNER_W), ("unbind", SysPerms::OWNER_W)],
        )?;
        let handle = Arc::new_cyclic(|weak| DriverHandle {
            driver,
            dir: dir.clone(),
            bound: Mutex::new(Vec::new()),
            bus: self.weak.clone(),
            is_registered: AtomicBool::new(true),
            weak: weak.clone(),
        });
        dir.set_ops(Arc::new(DriverDirOps {
            driver: handle.weak.clone(),
        }));
        self.drivers_dir.attach_child(dir)?;
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
        // No new bind starts from here on, and any bind already in flight
        // finishes before `unbind_from` takes the same device's binding lock.
        driver.is_registered.store(false, Ordering::Relaxed);
        for dev in self.devices() {
            let _ = self.unbind_from(&dev, driver);
        }
        // A device that is being removed has already left the bus list but may
        // not be unbound yet, so the driver's own list is drained as well.
        for dev in driver.devices() {
            let _ = self.unbind_from(&dev, driver);
        }
        let _ = self.drivers_dir.detach_child(driver.name());
        Ok(())
    }

    /// Returns whether new devices and drivers are matched automatically.
    pub fn is_autoprobe_enabled(&self) -> bool {
        self.autoprobe.load(Ordering::Relaxed)
    }

    /// Enables or disables automatic matching.
    pub fn set_autoprobe(&self, on: bool) {
        self.autoprobe.store(on, Ordering::Relaxed);
    }

    /// Offers an unbound device to every driver until one accepts it.
    pub fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
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

    /// Binds a specific driver to a device, if the bus matches them.
    pub fn bind(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        self.try_bind(dev, driver)
    }

    fn try_bind(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        if !self.bus.matches(dev.payload(), driver.match_data()) {
            return Err(Error::NoDriver);
        }
        let _guard = dev.bind_lock().lock();
        if !dev.base().is_added() {
            return Err(Error::NotAdded);
        }
        if !driver.is_registered.load(Ordering::Relaxed) {
            return Err(Error::DriverUnregistered);
        }
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        // The links come first so that `probe` sees the device as Linux's
        // drivers do; they are removed again if `probe` declines.
        add_link(
            driver.dir.as_ref(),
            dev.base().name(),
            &dev.base().tree_path(),
        )?;
        if let Err(e) = add_link(dev.base(), "driver", &SysObj::path(driver.dir.as_ref())) {
            remove_link(driver.dir.as_ref(), dev.base().name());
            return Err(e);
        }
        if let Err(e) = driver.probe(dev) {
            remove_driver_links(dev, driver);
            return Err(match e {
                Error::NoDriver => Error::NoDriver,
                _ => Error::ProbeFailed,
            });
        }
        if let Err(e) = dev.add_attrs(driver.dev_attrs()) {
            driver.remove(dev);
            remove_driver_links(dev, driver);
            return Err(e);
        }
        dev.set_driver(driver.clone());
        driver.bound.lock().push(dev.clone());
        Ok(())
    }

    /// Unbinds the device from its driver.
    pub fn unbind(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        self.unbind_inner(dev, None)
    }

    /// Unbinds the device, but only if `expected` is the driver bound to it.
    ///
    /// The identity check happens under the binding lock, so a concurrent
    /// rebind cannot make this tear down a driver the caller never named.
    pub fn unbind_from(
        &self,
        dev: &Arc<BusDevice<B>>,
        expected: &Arc<DriverHandle<B>>,
    ) -> Result<()> {
        self.unbind_inner(dev, Some(expected))
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
        // The links go first, then the driver's attribute files, then its
        // `remove`, which is the order of Linux's `__device_release_driver`,
        // so that nothing user space can open outlives the driver's hold on
        // the device.
        remove_driver_links(dev, &driver);
        dev.remove_attrs(driver.dev_attrs());
        driver.remove(dev);
        dev.take_driver();
        driver.bound.lock().retain(|d| !Arc::ptr_eq(d, dev));
        Ok(())
    }
}

/// Removes the `driver` link of `dev` and the device entry in the driver's
/// directory, the two links [`BusHandle::try_bind`] creates.
fn remove_driver_links<B: Bus>(dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) {
    remove_link(dev.base(), "driver");
    remove_link(driver.dir.as_ref(), dev.base().name());
}

/// A registered driver. Dereferences to the driver itself.
pub struct DriverHandle<B: Bus> {
    driver: Arc<dyn Driver<B>>,
    dir: Arc<Dir>,
    bound: Mutex<Vec<Arc<BusDevice<B>>>>,
    bus: Weak<BusHandle<B>>,
    /// Cleared by `unregister_driver`, so that a bind racing with it fails.
    is_registered: AtomicBool,
    weak: Weak<Self>,
}

impl<B: Bus> core::fmt::Debug for DriverHandle<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DriverHandle")
            .field("name", &self.driver.name())
            .finish()
    }
}

impl<B: Bus> Deref for DriverHandle<B> {
    type Target = dyn Driver<B>;

    fn deref(&self) -> &dyn Driver<B> {
        self.driver.as_ref()
    }
}

impl<B: Bus> DriverHandle<B> {
    /// Returns the devices bound to this driver.
    pub fn devices(&self) -> Vec<Arc<BusDevice<B>>> {
        self.bound.lock().clone()
    }

    /// Returns the bus the driver is registered with.
    pub fn bus(&self) -> Option<Arc<BusHandle<B>>> {
        self.bus.upgrade()
    }
}

/// The `bind` and `unbind` files of a driver directory.
struct DriverDirOps<B: Bus> {
    driver: Weak<DriverHandle<B>>,
}

impl<B: Bus> DirAttrOps for DriverDirOps<B> {
    fn store(&self, name: &str, value: &str) -> Result<()> {
        let driver = self.driver.upgrade().ok_or(Error::NotFound)?;
        let bus = driver.bus().ok_or(Error::NotFound)?;
        let dev = bus.find_device(value.trim()).ok_or(Error::NotFound)?;
        match name {
            "bind" => bus.bind(&dev, &driver),
            "unbind" => bus.unbind_from(&dev, &driver),
            _ => Err(Error::NotFound),
        }
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
                // Linux's `drivers_probe_store` reports success when the
                // device is already bound or when no driver matches.
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
        let Some(dev) = dev.as_any().downcast_ref::<BusDevice<B>>() else {
            // Only a `BusDevice<B>` reports this bus as its subsystem.
            ostd::error!("a device of another type was added to bus {}", B::NAME);
            return;
        };
        let dev = dev.this();
        self.devices.lock().push(dev.clone());
        if self.autoprobe.load(Ordering::Relaxed) {
            // No driver is not an error at registration time.
            let _ = self.probe(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let Some(dev) = dev.as_any().downcast_ref::<BusDevice<B>>() else {
            ostd::error!("a device of another type was removed from bus {}", B::NAME);
            return;
        };
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
