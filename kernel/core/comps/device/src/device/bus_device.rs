// SPDX-License-Identifier: MPL-2.0

//! Devices enumerated on a bus.

use alloc::{
    string::{String, ToString},
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Deref;

use ostd::sync::{Mutex, RwMutex};

use super::{
    AnyDevice, DeclaredParts, DevNode, DeviceBase, DeviceBuilder, DeviceInternals, Subsystem,
    impl_device_node,
};
use crate::{
    Result, SysStr,
    attr::{Attr, TyErasedAttr},
    bus::{Bus, BusHandle, DriverHandle},
    uevent::UeventVars,
};

/// A device enumerated on bus `B`.
///
/// Dereferences to the bus-specific payload `B::Device`.
pub struct BusDevice<B: Bus> {
    base: DeviceBase,
    bus: Arc<BusHandle<B>>,
    payload: B::Device,
    declared: DeclaredParts<Self>,
    driver: RwMutex<Option<Arc<DriverHandle<B>>>>,
    /// Serializes binding and unbinding, as Linux's `dev->mutex` does.
    bind_lock: Mutex<()>,
    weak: Weak<Self>,
}

/// Builds a [`BusDevice`].
pub type BusDeviceBuilder<B> = DeviceBuilder<Arc<BusHandle<B>>, <B as Bus>::Device, BusDevice<B>>;

impl<B: Bus> core::fmt::Debug for BusDevice<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BusDevice")
            .field("bus", &B::NAME)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl<B: Bus> Deref for BusDevice<B> {
    type Target = B::Device;

    fn deref(&self) -> &B::Device {
        &self.payload
    }
}

impl<B: Bus> BusDevice<B> {
    /// Starts building a device on `bus`.
    pub fn builder(
        bus: &Arc<BusHandle<B>>,
        name: impl Into<SysStr>,
        payload: B::Device,
    ) -> BusDeviceBuilder<B> {
        DeviceBuilder::new(bus.clone(), name.into(), payload)
    }

    /// Returns the bus this device is on.
    pub fn bus(&self) -> &Arc<BusHandle<B>> {
        &self.bus
    }

    /// Returns the bus-specific payload.
    pub fn payload(&self) -> &B::Device {
        &self.payload
    }

    /// Returns the driver currently bound, if any.
    pub fn driver(&self) -> Option<Arc<DriverHandle<B>>> {
        self.driver.read().clone()
    }

    /// Returns the lock that serializes binding and unbinding of this device.
    pub(crate) fn bind_lock(&self) -> &Mutex<()> {
        &self.bind_lock
    }

    /// Clears and returns the bound driver.
    pub(crate) fn take_driver(&self) -> Option<Arc<DriverHandle<B>>> {
        self.driver.write().take()
    }

    /// Records the bound driver.
    pub(crate) fn set_driver(&self, driver: Arc<DriverHandle<B>>) {
        *self.driver.write() = Some(driver);
    }

    /// Returns a strong reference to this device.
    pub(crate) fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }

    /// Adds a driver's attributes at bind time.
    pub(crate) fn add_attrs(&self, attrs: &[Attr<Self>]) -> Result<()> {
        self.base.attrs.add(TyErasedAttr::from_typed_slice(attrs))
    }

    /// Removes a driver's attributes at unbind time.
    pub(crate) fn remove_attrs(&self, attrs: &[Attr<Self>]) {
        let names: Vec<&'static str> = attrs.iter().map(Attr::name).collect();
        self.base.attrs.remove(&names);
    }
}

impl<B: Bus> BusDeviceBuilder<B> {
    /// Builds the device. It is not registered until
    /// [`add`](super::add) is called.
    pub fn build(self) -> Arc<BusDevice<B>> {
        let declared = self.declared_parts();
        Arc::new_cyclic(|weak: &Weak<BusDevice<B>>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            BusDevice {
                base: DeviceBase::new(self.name, self.parent, self.devnum, weak_self),
                bus: self.handle,
                payload: self.payload,
                declared,
                driver: RwMutex::new(None),
                bind_lock: Mutex::new(()),
                weak: weak.clone(),
            }
        })
    }
}

impl<B: Bus> AnyDevice for BusDevice<B> {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn subsystem(&self) -> Subsystem {
        Subsystem::bus(self.bus.clone())
    }

    fn driver_name(&self) -> Option<String> {
        self.driver.read().as_ref().map(|d| d.name().to_string())
    }
}

impl<B: Bus> DeviceInternals for BusDevice<B> {
    fn type_name(&self) -> Option<&'static str> {
        self.declared.type_name()
    }

    fn attr_groups(&self) -> Vec<TyErasedAttr> {
        self.declared.attr_groups(self.bus.bus().dev_attrs())
    }

    fn subsystem_uevent(&self, vars: &mut UeventVars) {
        self.bus.bus().uevent(self, vars);
        self.declared.type_uevent(self, vars);
    }

    fn devnode_override(&self) -> Option<DevNode> {
        self.declared.type_devnode(self)
    }
}

impl_device_node!(BusDevice<B: Bus>);
