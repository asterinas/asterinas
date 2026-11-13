// SPDX-License-Identifier: MPL-2.0

//! Driver binding and removal tests.

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::prelude::ktest;

use super::{
    toy::{self, ToyBus, ToyDev, ToyMatch},
    utils,
};
use crate::{
    bus::{BusDevice, Driver},
    common::{Attr, Result},
};

struct SuppressedDriver {
    name: &'static str,
    match_data: ToyMatch,
    attrs: &'static [Attr<BusDevice<ToyBus>>],
    probes: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
}

impl Driver<ToyBus> for SuppressedDriver {
    fn name(&self) -> &str {
        self.name
    }
    fn match_data(&self) -> &ToyMatch {
        &self.match_data
    }
    fn suppress_bind_attrs(&self) -> bool {
        true
    }
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<ToyBus>>] {
        self.attrs
    }
    fn on_probe(&self, _dev: &Arc<BusDevice<ToyBus>>) -> Result<()> {
        self.probes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn on_release(&self, _dev: &Arc<BusDevice<ToyBus>>) {
        self.releases.fetch_add(1, Ordering::Relaxed);
    }
}

#[ktest]
fn suppressed_bind_attrs_preserve_automatic_and_internal_binding() {
    // 1. Register a driver that suppresses bind/unbind attributes and check their absence.
    let (_, bus) = toy::register_toy_subsystems();
    let probes = Arc::new(AtomicUsize::new(0));
    let releases = Arc::new(AtomicUsize::new(0));
    let driver = bus
        .register_driver(Box::new(SuppressedDriver {
            name: "suppressed",
            match_data: ToyMatch { vendor: 101 },
            attrs: &[],
            probes: probes.clone(),
            releases: releases.clone(),
        }))
        .unwrap();
    let path = "/bus/toy/drivers/suppressed";
    let attrs = utils::attr_names(path);
    assert!(!attrs.iter().any(|name| name == "bind" || name == "unbind"));

    // 2. Add a matching device and check automatic binding and both driver links.
    let dev = BusDevice::builder(
        &bus,
        "suppressed0",
        ToyDev {
            vendor: 101,
            model: 0,
        },
    )
    .build();
    crate::add_device(&dev).unwrap();
    assert_eq!(probes.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(&dev.driver().unwrap(), &driver));
    assert!(utils::lookup("/devices/suppressed0/driver").is_some());
    assert!(utils::lookup("/bus/toy/drivers/suppressed/suppressed0").is_some());

    // 3. Writes to the absent attributes must fail without changing the binding.
    let directory = utils::lookup(path).unwrap().cast_to_node().unwrap();
    for name in ["bind", "unbind"] {
        assert!(matches!(
            directory.store_attr(name, "suppressed0"),
            Err(aster_systree::Error::NotFound)
        ));
    }
    assert!(Arc::ptr_eq(&dev.driver().unwrap(), &driver));

    // 4. Unbind through the kernel API and check that the release callback runs.
    bus.unbind(&dev).unwrap();
    assert!(dev.driver().is_none());
    assert_eq!(releases.load(Ordering::Relaxed), 1);

    // 5. Rebind through the kernel API and check that the probe callback runs again.
    bus.bind(&dev, &driver).unwrap();
    assert!(Arc::ptr_eq(&dev.driver().unwrap(), &driver));
    assert_eq!(probes.load(Ordering::Relaxed), 2);

    // 6. Unbind, remove the device, and unregister the driver.
    bus.unbind(&dev).unwrap();
    crate::remove_device(&dev).unwrap();
    bus.unregister_driver(&driver).unwrap();
}

#[ktest]
fn suppressed_bind_attrs_allow_device_removal_and_driver_unregistration() {
    // 1. Add a device before registering its driver.
    let (_, bus) = toy::register_toy_subsystems();
    let releases = Arc::new(AtomicUsize::new(0));
    let dev = BusDevice::builder(
        &bus,
        "suppressed-removal0",
        ToyDev {
            vendor: 103,
            model: 0,
        },
    )
    .build();
    crate::add_device(&dev).unwrap();

    // 2. Register the driver and check that it binds the existing device.
    let driver = bus
        .register_driver(Box::new(SuppressedDriver {
            name: "suppressed-removal",
            match_data: ToyMatch { vendor: 103 },
            attrs: &[],
            probes: Arc::new(AtomicUsize::new(0)),
            releases: releases.clone(),
        }))
        .unwrap();
    assert!(Arc::ptr_eq(&dev.driver().unwrap(), &driver));

    // 3. Remove the bound device and check that its driver releases it.
    crate::remove_device(&dev).unwrap();
    assert_eq!(releases.load(Ordering::Relaxed), 1);
    assert!(dev.driver().is_none());
    assert!(utils::lookup("/devices/suppressed-removal0").is_none());

    // 4. Add another device and check that the driver can still bind it.
    let second = BusDevice::builder(
        &bus,
        "suppressed-removal1",
        ToyDev {
            vendor: 103,
            model: 1,
        },
    )
    .build();
    crate::add_device(&second).unwrap();
    assert!(Arc::ptr_eq(&second.driver().unwrap(), &driver));

    // 5. Unregister the driver and check that the remaining device is released.
    bus.unregister_driver(&driver).unwrap();
    assert_eq!(releases.load(Ordering::Relaxed), 2);
    assert!(second.driver().is_none());
    assert!(utils::lookup("/bus/toy/drivers/suppressed-removal").is_none());
    assert!(utils::lookup("/devices/suppressed-removal1/driver").is_none());

    // 6. Remove the remaining unbound device.
    crate::remove_device(&second).unwrap();
}
