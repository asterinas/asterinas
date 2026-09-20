// SPDX-License-Identifier: MPL-2.0

//! Drivers for devices on a bus.
//!
//! A [`Driver`] supplies the matching data and callbacks used by [`BusHandle`].
//! Registering it creates a [`DriverHandle`],
//! which tracks its binding state and provides the driver's sysfs attribute directory.

use alloc::{
    boxed::Box,
    string::ToString,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Deref;

use aster_systree::SysPerms;
use ostd::sync::Mutex;

use crate::{
    Error, Result, SysStr,
    attr::Attr,
    bus::{Bus, BusHandle},
    device::{AnyDevice, BusDevice},
    node::{self, Dir, DirAttrOps},
};

/// A driver for devices on bus `B`.
///
/// The device model holds the device's binding lock when invoking
/// [`probe`](Self::probe), [`remove`](Self::remove),
/// and the attribute callbacks returned by [`dev_attrs`](Self::dev_attrs).
/// While running under this lock, callbacks must not synchronously perform any of the following operations,
/// which can wait for the same lock and deadlock:
///
/// - bind, unbind, or remove this device;
/// - unregister this driver;
/// - invoke a driver attribute callback on this device.
///
/// Registering and removing child devices is allowed.
pub trait Driver<B: Bus>: Send + Sync + 'static {
    /// The driver name: the directory under `/sys/bus/<bus>/drivers`.
    ///
    /// Must be a valid node name according to [`aster_systree::is_valid_name`].
    fn name(&self) -> &str;

    /// The devices this driver accepts, in the bus's terms.
    fn match_data(&self) -> &B::MatchData;

    /// Takes over a matched device.
    /// Return an error to decline it.
    fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()>;

    /// Releases a device before it is unbound or removed.
    fn remove(&self, _dev: &Arc<BusDevice<B>>) {}

    /// Attributes every bound device gets while it is bound.
    ///
    /// Attribute callbacks hold the device's binding lock,
    /// so they finish before [`remove`](Self::remove) releases its resources.
    /// The callback restrictions in [`Driver`] also apply here.
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<B>>] {
        &[]
    }
}

/// A registered driver.
/// Dereferences to the driver itself.
pub struct DriverHandle<B: Bus> {
    driver: Arc<dyn Driver<B>>,
    // Keep a reference so device attributes remain fixed for this registration.
    dev_attrs: &'static [Attr<BusDevice<B>>],
    dir: Arc<Dir>,
    state: Mutex<DriverState<B>>,
    bus: Weak<BusHandle<B>>,
    weak: Weak<Self>,
}

impl<B: Bus> DriverHandle<B> {
    /// Creates a driver handle and its attribute directory.
    ///
    /// # Panics
    ///
    /// Panics if the driver name is not a valid `SysTree` node name.
    pub(crate) fn new(driver: Arc<dyn Driver<B>>, bus: Weak<BusHandle<B>>) -> Result<Arc<Self>> {
        let dir = Dir::with_attrs(
            SysStr::from(driver.name().to_string()),
            &[("bind", SysPerms::OWNER_W), ("unbind", SysPerms::OWNER_W)],
        )?;
        let dev_attrs = driver.dev_attrs();
        let handle = Arc::new_cyclic(|weak| DriverHandle {
            driver,
            dev_attrs,
            dir: dir.clone(),
            state: Mutex::new(DriverState::new()),
            bus,
            weak: weak.clone(),
        });
        dir.set_ops(Box::new(DriverDirOps {
            driver: handle.weak.clone(),
        }));
        Ok(handle)
    }

    /// Returns the devices bound to this driver.
    ///
    /// Returns an empty list once unregistration starts.
    pub fn devices(&self) -> Vec<Arc<BusDevice<B>>> {
        self.state.lock().bound_devices().to_vec()
    }

    /// Returns the bus the driver is registered with.
    pub fn bus(&self) -> Option<Arc<BusHandle<B>>> {
        self.bus.upgrade()
    }

    /// Returns the device attributes saved when the driver was registered.
    pub(crate) fn dev_attrs(&self) -> &'static [Attr<BusDevice<B>>] {
        self.dev_attrs
    }

    /// Returns the driver's attribute directory.
    pub(crate) fn dir(&self) -> &Arc<Dir> {
        &self.dir
    }

    /// Returns the lock protecting registration and binding records.
    pub(crate) fn state(&self) -> &Mutex<DriverState<B>> {
        &self.state
    }
}

impl<B: Bus> Deref for DriverHandle<B> {
    type Target = dyn Driver<B>;

    fn deref(&self) -> &dyn Driver<B> {
        self.driver.as_ref()
    }
}

impl<B: Bus> core::fmt::Debug for DriverHandle<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DriverHandle")
            .field("name", &self.driver.name())
            .finish()
    }
}

/// A driver's registration state includes its binding records.
/// Checking that the driver is registered and recording a binding share one lock,
/// so unregistration cannot miss a probe that has been allowed to run.
///
/// Lock order: device binding lock, then driver state lock.
/// Never hold the driver state lock across callbacks or while acquiring a device binding lock.
pub(crate) enum DriverState<B: Bus> {
    /// Accepts bindings; successful bindings move from `binding` to `bound` under the state lock.
    Registered {
        /// Successfully bound devices, retained until unbinding finishes.
        bound: Vec<Arc<BusDevice<B>>>,
        /// Devices being bound, retained until success or failed-bind cleanup.
        binding: Vec<Arc<BusDevice<B>>>,
    },
    /// No new bindings are allowed; cleanup of existing bindings may still be in progress.
    Unregistered,
}

impl<B: Bus> DriverState<B> {
    /// Records the start of a binding attempt.
    pub(crate) fn start_binding(&mut self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        let Self::Registered { binding, .. } = self else {
            return Err(Error::DriverUnregistered);
        };
        binding.push(dev.clone());
        Ok(())
    }

    /// Moves a successfully bound device from `binding` to `bound`.
    pub(crate) fn finish_binding(&mut self, dev: &Arc<BusDevice<B>>) {
        // Unregistration may already own the records and be waiting to unbind.
        if let Self::Registered { bound, binding } = self {
            binding.retain(|d| !Arc::ptr_eq(d, dev));
            bound.push(dev.clone());
        }
    }

    /// Removes the record of a failed binding attempt.
    pub(crate) fn abort_binding(&mut self, dev: &Arc<BusDevice<B>>) {
        if let Self::Registered { binding, .. } = self {
            binding.retain(|d| !Arc::ptr_eq(d, dev));
        }
    }

    /// Removes a device from the bound list.
    pub(crate) fn forget_bound(&mut self, dev: &Arc<BusDevice<B>>) {
        if let Self::Registered { bound, .. } = self {
            bound.retain(|d| !Arc::ptr_eq(d, dev));
        }
    }

    /// Sets the state to `Unregistered` and returns all bound and binding devices.
    pub(crate) fn set_unregistered(&mut self) -> Result<Vec<Arc<BusDevice<B>>>> {
        match core::mem::replace(self, Self::Unregistered) {
            Self::Registered { mut bound, binding } => {
                bound.extend(binding);
                Ok(bound)
            }
            Self::Unregistered => Err(Error::NotFound),
        }
    }

    /// Creates a registered state with no devices.
    fn new() -> Self {
        Self::Registered {
            bound: Vec::new(),
            binding: Vec::new(),
        }
    }

    /// Returns the bound devices still owned by this state.
    fn bound_devices(&self) -> &[Arc<BusDevice<B>>] {
        match self {
            Self::Registered { bound, .. } => bound,
            Self::Unregistered => &[],
        }
    }
}

/// Removes the `driver` link of `dev` and the device entry in the driver's directory,
/// the two links created during binding.
pub(crate) fn remove_driver_links<B: Bus>(dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) {
    node::remove_link(dev.base(), "driver");
    node::remove_link(driver.dir.as_ref(), dev.base().name());
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
