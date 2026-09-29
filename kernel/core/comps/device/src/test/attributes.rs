// SPDX-License-Identifier: MPL-2.0

//! Device attribute tests.

use alloc::sync::Arc;

use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use super::{
    toy::{self, ToyBlock, ToyBus, ToyDev, ToyDisk, ToyDiskDriver, ToyMatch},
    utils,
};
use crate::{
    bus::BusDevice,
    class::ClassDevice,
    common::{Attr, DevNum, Error},
};

/// A driver's attributes come and go, and the published attribute set is rebuilt each time.
/// The attributes that survive must keep their IDs,
/// because sysfs derives an inode number from `(node id, attribute id)`:
/// renumbering them would give every file in a device's directory a new inode
/// whenever a driver bound beside it.
#[ktest]
fn attribute_ids_survive_bind_and_unbind() {
    let (class, bus) = toy::register_toy_subsystems();

    // 1. Add an unbound device and record its attribute IDs.
    // Vendor 9 matches no driver registered by another test.
    let dev = BusDevice::builder(
        &bus,
        "ids0",
        ToyDev {
            vendor: 9,
            model: 91,
        },
    )
    .build();
    crate::add_device(&dev).unwrap();
    let path = "/devices/ids0";
    let before = utils::attr_ids(path);
    assert!(!before.is_empty());
    assert!(before.iter().all(|(name, _)| name != "bound_by"));

    // 2. Register a matching driver and check that binding preserves existing IDs.
    let driver = bus
        .register_driver(Arc::new(ToyDiskDriver {
            match_data: ToyMatch { vendor: 9 },
            class: class.clone(),
        }))
        .unwrap();
    assert!(dev.driver().is_some());

    let bound = utils::attr_ids(path);
    for (name, id) in &before {
        assert_eq!(
            bound.iter().find(|(n, _)| n == name).map(|(_, i)| *i),
            Some(*id),
            "attribute `{}` was renumbered by a bind",
            name
        );
    }
    let bound_by_id = bound
        .iter()
        .find(|(n, _)| n == "bound_by")
        .map(|(_, i)| *i)
        .expect("the driver's attribute is present while it is bound");
    assert!(before.iter().all(|(_, id)| *id != bound_by_id));

    // 3. Unbind and check that the original attributes and IDs remain.
    bus.unbind(&dev).unwrap();
    assert_eq!(utils::attr_ids(path), before);

    // 4. Rebind and check that the driver attribute reuses its ID.
    bus.bind(&dev, &driver).unwrap();
    assert_eq!(
        utils::attr_ids(path)
            .iter()
            .find(|(n, _)| n == "bound_by")
            .map(|(_, i)| *i),
        Some(bound_by_id)
    );

    // 5. Unbind, remove the device, and unregister the driver.
    bus.unbind(&dev).unwrap();
    crate::remove_device(&dev).unwrap();
    bus.unregister_driver(&driver).unwrap();
}

/// Registration hands the core, subsystem, device type, and own attributes over as one batch,
/// so a layer that reuses a name the core already took must be rejected there.
/// Otherwise its callback would quietly replace the core's while sysfs went on showing the core's
/// entry.
#[ktest]
fn an_attribute_may_not_shadow_a_core_one() {
    // 1. Define an attribute that conflicts with the core device-number attribute.
    const SHADOWING: &[Attr<BusDevice<ToyBus>>] = &[Attr::ro("dev", |_dev, w| {
        writeln!(w, "not the core's device number")?;
        Ok(())
    })];

    // 2. Build a device that supplies both a device number and this attribute.
    let (_, bus) = toy::register_toy_subsystems();
    let dev = BusDevice::builder(
        &bus,
        "shadow0",
        ToyDev {
            vendor: 8,
            model: 1,
        },
    )
    .devnum(DevNum::char(DeviceId::new(
        MajorId::new(200),
        MinorId::new(1),
    )))
    .attrs(SHADOWING)
    .build();

    // 3. Check that registration fails and leaves no device directory behind.
    assert!(matches!(crate::add_device(&dev), Err(Error::NameConflict)));
    assert!(utils::lookup("/devices/shadow0").is_none());
}

#[ktest]
fn invalid_attribute_names_are_rejected() {
    // 1. Define one attribute for each invalid name.
    const INVALID_ATTRS: &[[Attr<ClassDevice<ToyBlock>>; 1]] = &[
        [Attr::ro("", |_dev, _w| Ok(()))],
        [Attr::ro(".", |_dev, _w| Ok(()))],
        [Attr::ro("..", |_dev, _w| Ok(()))],
        [Attr::ro("bad/name", |_dev, _w| Ok(()))],
        [Attr::ro("bad\0name", |_dev, _w| Ok(()))],
    ];
    let (class, _) = toy::register_toy_subsystems();

    // 2. Check that registration rejects each attribute and removes the directory.
    for attrs in INVALID_ATTRS {
        let dev = ClassDevice::builder(&class, "invalid-attr", ToyDisk { sectors: 8 })
            .attrs(attrs)
            .build();
        assert!(matches!(
            crate::add_device(&dev),
            Err(Error::SysTree(aster_systree::Error::InvalidName))
        ));
        assert!(utils::lookup("/devices/virtual/toyblk/invalid-attr").is_none());
        assert!(utils::lookup("/class/toyblk/invalid-attr").is_none());
    }
}
