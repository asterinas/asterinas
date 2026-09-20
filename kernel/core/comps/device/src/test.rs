// SPDX-License-Identifier: MPL-2.0

//! Kernel-mode tests with the toy bus, block class, and drivers.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::{SysBranchNode, SysObj};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use crate::{
    AnyDevice, Attr, Bus, BusDevice, BusHandle, Class, ClassDevice, ClassHandle, ClassInterface,
    DevNode, DevNum, DeviceType, Driver, Error, Result,
};

/// A toy bus: devices carry a vendor and a model id, drivers accept a vendor.
struct ToyBus;

struct ToyDev {
    vendor: u32,
    model: u32,
}

struct ToyMatch {
    vendor: u32,
}

const TOY_DEV_ATTRS: &[Attr<BusDevice<ToyBus>>] = &[
    Attr::ro("vendor", |dev, w| {
        writeln!(w, "{:#06x}", dev.vendor)?;
        Ok(())
    }),
    Attr::ro("model", |dev, w| {
        writeln!(w, "{:#06x}", dev.model)?;
        Ok(())
    }),
];

impl Bus for ToyBus {
    const NAME: &'static str = "toy";
    type Device = ToyDev;
    type MatchData = ToyMatch;

    fn matches(&self, dev: &ToyDev, data: &ToyMatch) -> bool {
        dev.vendor == data.vendor
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        TOY_DEV_ATTRS
    }
}

static DISK_TYPE: DeviceType<BusDevice<ToyBus>> = DeviceType::named("disk");

/// A driver that creates a toy disk on probe.
struct ToyDiskDriver {
    match_data: ToyMatch,
    class: Arc<ClassHandle<ToyBlock>>,
}

const BOUND_BY: &[Attr<BusDevice<ToyBus>>] = &[Attr::ro("bound_by", |_dev, w| {
    writeln!(w, "toy_disk")?;
    Ok(())
})];

impl Driver<ToyBus> for ToyDiskDriver {
    fn name(&self) -> &str {
        "toy_disk"
    }

    fn match_data(&self) -> &ToyMatch {
        &self.match_data
    }

    fn probe(&self, dev: &Arc<BusDevice<ToyBus>>) -> Result<()> {
        let name = alloc::format!("td{}", dev.model);
        let disk = ClassDevice::builder(&self.class, name, ToyDisk { sectors: 8 })
            .parent(dev.clone())
            .devnum(DevNum::block(DeviceId::new(
                MajorId::new(200),
                MinorId::new(dev.model),
            )))
            .build();
        crate::add(&disk)?;
        Ok(())
    }

    fn remove(&self, dev: &Arc<BusDevice<ToyBus>>) {
        // The core removes the driver's files and links before calling this,
        // as Linux's `__device_release_driver` does.
        let path = dev.path().to_string();
        assert!(lookup(&alloc::format!("{path}/driver")).is_none());
        assert!(!attr_names(&path).iter().any(|name| name == "bound_by"));

        for child in dev.base().child_devices() {
            crate::remove(&child).unwrap();
        }
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<ToyBus>>] {
        BOUND_BY
    }
}

/// A toy block class.
struct ToyBlock;

struct ToyDisk {
    sectors: u64,
}

const TOY_BLOCK_ATTRS: &[Attr<ClassDevice<ToyBlock>>] = &[Attr::ro("size", |dev, w| {
    writeln!(w, "{}", dev.sectors)?;
    Ok(())
})];

impl Class for ToyBlock {
    const NAME: &'static str = "toyblk";
    type Device = ToyDisk;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        TOY_BLOCK_ATTRS
    }

    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        Some(DevNode {
            path: None,
            mode: Some(0o660),
        })
    }
}

struct CountingInterface {
    added: AtomicUsize,
    removed: AtomicUsize,
}

impl ClassInterface<ToyBlock> for CountingInterface {
    fn add_dev(&self, _dev: &Arc<ClassDevice<ToyBlock>>) {
        self.added.fetch_add(1, Ordering::Relaxed);
    }

    fn remove_dev(&self, _dev: &Arc<ClassDevice<ToyBlock>>) {
        self.removed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Resolves a path below the sysfs root, following no symlinks.
fn lookup(path: &str) -> Option<Arc<dyn SysObj>> {
    let mut node: Arc<dyn SysBranchNode> = aster_systree::primary_tree().root().clone();
    let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
    while let Some(part) = parts.next() {
        let child = node.child(part)?;
        if parts.peek().is_none() {
            return Some(child);
        }
        node = child.cast_to_branch()?;
    }
    Some(node)
}

/// Returns the target path stored in the symlink at `path`, without resolving it.
fn link_target(path: &str) -> Option<String> {
    lookup(path)?
        .cast_to_symlink()
        .map(|l| l.target_path().to_string())
}

fn read_attr(path: &str, name: &str) -> String {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.show_attr(name).unwrap()
}

fn attr_ids(path: &str) -> Vec<(String, u8)> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .iter()
        .map(|a| (a.name().to_string(), a.id()))
        .collect()
}

fn attr_names(path: &str) -> Vec<String> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .iter()
        .map(|a| a.name().to_string())
        .collect()
}

type ToySubsystems = (Arc<ClassHandle<ToyBlock>>, Arc<BusHandle<ToyBus>>);

fn register_toy_subsystems() -> ToySubsystems {
    static TOY_SUBSYSTEMS: spin::Once<ToySubsystems> = spin::Once::new();

    crate::init_for_ktest();
    TOY_SUBSYSTEMS
        .call_once(|| {
            let bus = crate::register_bus(ToyBus).unwrap();
            let class = crate::register_class(ToyBlock).unwrap();
            (class, bus)
        })
        .clone()
}

#[ktest]
fn roots_exist() {
    // 1. Register the toy bus and class.
    register_toy_subsystems();

    // 2. Check the root directories and subsystem directories.
    for path in [
        "/devices",
        "/devices/virtual",
        "/bus",
        "/class",
        "/dev/char",
        "/dev/block",
        "/bus/toy/devices",
        "/bus/toy/drivers",
        "/class/toyblk",
    ] {
        assert!(lookup(path).is_some(), "{path} missing");
    }

    // 3. Check that automatic probing is enabled by default.
    assert_eq!(read_attr("/bus/toy", "drivers_autoprobe"), "1\n");
}

#[ktest]
fn bus_device_registration_and_probe() {
    // 1. Register the driver and an interface that counts class notifications.
    let (class, bus) = register_toy_subsystems();
    let driver = bus
        .register_driver(Arc::new(ToyDiskDriver {
            match_data: ToyMatch { vendor: 1 },
            class: class.clone(),
        }))
        .unwrap();
    let iface = Arc::new(CountingInterface {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    class
        .register_interface(iface.clone() as Arc<dyn ClassInterface<ToyBlock>>)
        .unwrap();

    // 2. Add a root and a matching bus device to trigger the driver probe.
    let root = crate::BareDevice::new_root("toy0000:00");
    crate::add(&root).unwrap();
    let bus_dev = BusDevice::builder(
        &bus,
        "0000:00:01.0",
        ToyDev {
            vendor: 1,
            model: 3,
        },
    )
    .parent(root.clone())
    .dev_type(&DISK_TYPE)
    .build();
    crate::add(&bus_dev).unwrap();

    // 3. Check the bus device placement and subsystem links.
    assert_eq!(bus_dev.path(), "/devices/toy0000:00/0000:00:01.0");
    assert_eq!(
        link_target("/devices/toy0000:00/0000:00:01.0/subsystem").unwrap(),
        "../../../bus/toy"
    );
    assert_eq!(
        link_target("/bus/toy/devices/0000:00:01.0").unwrap(),
        "../../../devices/toy0000:00/0000:00:01.0"
    );

    // 4. Check the driver binding and links in both directions.
    assert!(bus_dev.driver().is_some_and(|d| Arc::ptr_eq(&d, &driver)));
    assert_eq!(
        link_target("/devices/toy0000:00/0000:00:01.0/driver").unwrap(),
        "../../../bus/toy/drivers/toy_disk"
    );
    assert_eq!(
        link_target("/bus/toy/drivers/toy_disk/0000:00:01.0").unwrap(),
        "../../../../devices/toy0000:00/0000:00:01.0"
    );

    // 5. Check the attributes supplied by the bus and driver.
    let names = attr_names("/devices/toy0000:00/0000:00:01.0");
    for expected in ["vendor", "model", "bound_by"] {
        assert!(names.iter().any(|n| n == expected), "{expected} missing");
    }
    assert_eq!(
        read_attr("/devices/toy0000:00/0000:00:01.0", "vendor"),
        "0x0001\n"
    );
    assert_eq!(names.len(), 3);

    // 6. Check the disk created by probe, its links, attributes, and notification.
    let disk_path = "/devices/toy0000:00/0000:00:01.0/toyblk/td3";
    assert!(lookup(disk_path).is_some(), "class device missing");
    assert_eq!(
        link_target(&alloc::format!("{disk_path}/device")).unwrap(),
        "../../../0000:00:01.0"
    );
    assert_eq!(
        link_target(&alloc::format!("{disk_path}/subsystem")).unwrap(),
        "../../../../../class/toyblk"
    );
    assert_eq!(
        link_target("/class/toyblk/td3").unwrap(),
        "../../devices/toy0000:00/0000:00:01.0/toyblk/td3"
    );
    assert_eq!(
        link_target("/dev/block/200:3").unwrap(),
        "../../devices/toy0000:00/0000:00:01.0/toyblk/td3"
    );
    assert_eq!(read_attr(disk_path, "dev"), "200:3\n");
    assert_eq!(read_attr(disk_path, "size"), "8\n");
    assert_eq!(attr_names(disk_path).len(), 2);
    assert_eq!(iface.added.load(Ordering::Relaxed), 1);

    // 7. Unbind and check that the disk, driver entries, and glue directory vanish.
    bus.unbind(&bus_dev).unwrap();
    assert!(bus_dev.driver().is_none());
    assert!(lookup("/devices/toy0000:00/0000:00:01.0/driver").is_none());
    assert!(lookup("/bus/toy/drivers/toy_disk/0000:00:01.0").is_none());
    assert!(
        !attr_names("/devices/toy0000:00/0000:00:01.0")
            .iter()
            .any(|n| n == "bound_by")
    );
    assert!(lookup(disk_path).is_none());
    assert!(lookup("/devices/toy0000:00/0000:00:01.0/toyblk").is_none());
    assert!(lookup("/class/toyblk/td3").is_none());
    assert!(lookup("/dev/block/200:3").is_none());
    assert_eq!(iface.removed.load(Ordering::Relaxed), 1);

    // 8. Rebind through sysfs and check that the disk is recreated.
    let driver_dir = lookup("/bus/toy/drivers/toy_disk")
        .unwrap()
        .cast_to_node()
        .unwrap();
    driver_dir.store_attr("bind", "0000:00:01.0\n").unwrap();
    assert!(bus_dev.driver().is_some());
    assert!(lookup(disk_path).is_some());

    // 9. Reject removal while the disk exists, then unbind and remove the device.
    assert!(matches!(crate::remove(&bus_dev), Err(Error::HasChildren)));
    driver_dir.store_attr("unbind", "0000:00:01.0\n").unwrap();
    crate::remove(&bus_dev).unwrap();
    assert!(lookup("/devices/toy0000:00/0000:00:01.0").is_none());
    assert!(lookup("/bus/toy/devices/0000:00:01.0").is_none());
    assert!(matches!(crate::remove(&bus_dev), Err(Error::NotAdded)));
    assert!(matches!(crate::add(&bus_dev), Err(Error::AlreadyAdded)));

    // 10. Remove the root and unregister the driver and interface.
    crate::remove(&root).unwrap();
    bus.unregister_driver(&driver).unwrap();
    class
        .unregister_interface(&(iface as Arc<dyn ClassInterface<ToyBlock>>))
        .unwrap();
}

#[ktest]
fn parentless_class_device_lives_under_virtual() {
    // 1. Add a class device without a parent.
    let (class, _) = register_toy_subsystems();
    let dev = ClassDevice::builder(&class, "loop0", ToyDisk { sectors: 1 })
        .devnum(DevNum::block(DeviceId::new(
            MajorId::new(7),
            MinorId::new(0),
        )))
        .build();
    crate::add(&dev).unwrap();

    // 2. Check placement under virtual and the links to its class.
    assert_eq!(dev.path(), "/devices/virtual/toyblk/loop0");
    assert_eq!(
        link_target("/class/toyblk/loop0").unwrap(),
        "../../devices/virtual/toyblk/loop0"
    );
    assert_eq!(
        link_target("/devices/virtual/toyblk/loop0/subsystem").unwrap(),
        "../../../../class/toyblk"
    );
    assert!(lookup("/devices/virtual/toyblk/loop0/device").is_none());

    // 3. Remove the device and check that the empty glue directory disappears.
    crate::remove(&dev).unwrap();
    assert!(lookup("/devices/virtual/toyblk").is_none());
}

#[ktest]
fn class_device_under_class_device_has_no_glue_dir() {
    // 1. Add a disk to serve as the parent.
    let (class, _) = register_toy_subsystems();
    let disk = ClassDevice::builder(&class, "sd0", ToyDisk { sectors: 16 }).build();
    crate::add(&disk).unwrap();

    // 2. Add a partition and check that it sits directly under the disk.
    let part = ClassDevice::builder(&class, "sd0p1", ToyDisk { sectors: 8 })
        .parent(disk.clone())
        .build();
    crate::add(&part).unwrap();
    assert_eq!(part.path(), "/devices/virtual/toyblk/sd0/sd0p1");

    // 3. Remove the child before its parent.
    crate::remove(&part).unwrap();
    crate::remove(&disk).unwrap();
}

#[ktest]
fn invalid_states_are_rejected() {
    // 1. Reject a child whose parent has not been registered.
    let (class, _) = register_toy_subsystems();
    let parent = ClassDevice::builder(&class, "orphan-parent", ToyDisk { sectors: 0 }).build();
    let child = ClassDevice::builder(&class, "orphan", ToyDisk { sectors: 0 })
        .parent(parent.clone())
        .build();
    assert!(matches!(crate::add(&child), Err(Error::ParentNotAdded)));

    // 2. Reject a duplicate directory name without disturbing the first device.
    let a = ClassDevice::builder(&class, "twin", ToyDisk { sectors: 0 }).build();
    let b = ClassDevice::builder(&class, "twin", ToyDisk { sectors: 0 }).build();
    crate::add(&a).unwrap();
    assert!(matches!(crate::add(&b), Err(Error::NameConflict)));
    assert!(lookup("/devices/virtual/toyblk/twin").is_some());
    assert_eq!(
        link_target("/class/toyblk/twin").unwrap(),
        "../../devices/virtual/toyblk/twin"
    );
    crate::remove(&a).unwrap();
    assert!(lookup("/class/toyblk/twin").is_none());

    // 3. Put devices with the same name under different parents: the second
    // registration must fail on the class index and preserve the first entry.
    let p1 = ClassDevice::builder(&class, "p1", ToyDisk { sectors: 0 }).build();
    let p2 = ClassDevice::builder(&class, "p2", ToyDisk { sectors: 0 }).build();
    crate::add(&p1).unwrap();
    crate::add(&p2).unwrap();
    let c1 = ClassDevice::builder(&class, "same", ToyDisk { sectors: 0 })
        .parent(p1.clone())
        .build();
    let c2 = ClassDevice::builder(&class, "same", ToyDisk { sectors: 0 })
        .parent(p2.clone())
        .build();
    crate::add(&c1).unwrap();
    assert!(matches!(crate::add(&c2), Err(Error::NameConflict)));
    assert_eq!(
        link_target("/class/toyblk/same").unwrap(),
        "../../devices/virtual/toyblk/p1/same"
    );
    assert!(lookup("/devices/virtual/toyblk/p2/same").is_none());

    // 4. Remove the registered child and both parents.
    crate::remove(&c1).unwrap();
    crate::remove(&p2).unwrap();
    crate::remove(&p1).unwrap();
}

#[ktest]
fn failed_add_does_not_notify_interfaces() {
    // 1. Register an interface and record the notifications replayed to it.
    let (class, _) = register_toy_subsystems();
    let iface = Arc::new(CountingInterface {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    class
        .register_interface(iface.clone() as Arc<dyn ClassInterface<ToyBlock>>)
        .unwrap();
    let before_added = iface.added.load(Ordering::Relaxed);

    // 2. Add one device and reject a duplicate without extra notifications.
    let a = ClassDevice::builder(&class, "clash", ToyDisk { sectors: 0 }).build();
    let b = ClassDevice::builder(&class, "clash", ToyDisk { sectors: 0 }).build();
    crate::add(&a).unwrap();
    assert!(matches!(crate::add(&b), Err(Error::NameConflict)));
    assert_eq!(iface.added.load(Ordering::Relaxed), before_added + 1);
    assert_eq!(iface.removed.load(Ordering::Relaxed), 0);

    // 3. Remove the registered device and check for one removal notification.
    crate::remove(&a).unwrap();
    assert_eq!(iface.removed.load(Ordering::Relaxed), 1);

    // 4. Unregister the interface.
    class
        .unregister_interface(&(iface as Arc<dyn ClassInterface<ToyBlock>>))
        .unwrap();
}

/// A driver's attributes come and go, and the published attribute set is rebuilt each time.
/// The attributes that survive must keep their IDs,
/// because sysfs derives an inode number from `(node id, attribute id)`:
/// renumbering them would give every file in a device's directory a new inode
/// whenever a driver bound beside it.
#[ktest]
fn attribute_ids_survive_bind_and_unbind() {
    let (class, bus) = register_toy_subsystems();

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
    crate::add(&dev).unwrap();
    let path = "/devices/ids0";
    let before = attr_ids(path);
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

    let bound = attr_ids(path);
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
    assert_eq!(attr_ids(path), before);

    // 4. Rebind and check that the driver attribute reuses its ID.
    bus.bind(&dev, &driver).unwrap();
    assert_eq!(
        attr_ids(path)
            .iter()
            .find(|(n, _)| n == "bound_by")
            .map(|(_, i)| *i),
        Some(bound_by_id)
    );

    // 5. Unbind, remove the device, and unregister the driver.
    bus.unbind(&dev).unwrap();
    crate::remove(&dev).unwrap();
    bus.unregister_driver(&driver).unwrap();
}

/// Registration hands the core, subsystem, type and own attributes over as one batch,
/// so a layer that reuses a name the core already took must be rejected there.
/// Otherwise its callback would quietly replace the core's while sysfs went on showing the core's entry.
#[ktest]
fn an_attribute_may_not_shadow_a_core_one() {
    // 1. Define an attribute that conflicts with the core device-number attribute.
    const SHADOWING: &[Attr<BusDevice<ToyBus>>] = &[Attr::ro("dev", |_dev, w| {
        writeln!(w, "not the core's device number")?;
        Ok(())
    })];

    // 2. Build a device that supplies both a device number and this attribute.
    let (_, bus) = register_toy_subsystems();
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
    assert!(matches!(crate::add(&dev), Err(Error::NameConflict)));
    assert!(lookup("/devices/shadow0").is_none());
}

#[ktest]
fn valid_device_names_are_accepted() {
    // 1. Register the toy bus and class for constructing typed devices.
    let (class, bus) = register_toy_subsystems();

    // 2. Check ordinary names, punctuation, and the `!` used in device paths.
    for name in ["disk0", "0000:00:01.0", "disk.part-1", "cciss!c0d0"] {
        let bare_dev = crate::BareDevice::new_root(name);
        let bus_dev = BusDevice::builder(
            &bus,
            name,
            ToyDev {
                vendor: 1,
                model: 3,
            },
        )
        .build();
        let class_dev = ClassDevice::builder(&class, name, ToyDisk { sectors: 8 }).build();
        assert_eq!(bare_dev.base().name().as_ref(), name);
        assert_eq!(bus_dev.base().name().as_ref(), name);
        assert_eq!(class_dev.base().name().as_ref(), name);
    }
}

#[ktest]
#[should_panic]
fn empty_device_name_panics_at_construction() {
    crate::BareDevice::new_root("");
}

#[ktest]
#[should_panic]
fn device_name_with_slash_panics_at_construction() {
    crate::BareDevice::new_root("bad/name");
}

#[ktest]
#[should_panic]
fn device_name_with_nul_panics_at_construction() {
    crate::BareDevice::new_root("bad\0name");
}

#[ktest]
#[should_panic]
fn dot_bus_device_name_panics_at_build() {
    // 1. Obtain the registered toy bus.
    let (_, bus) = register_toy_subsystems();

    // 2. Building the device must reject the name before registration.
    BusDevice::builder(
        &bus,
        ".",
        ToyDev {
            vendor: 1,
            model: 3,
        },
    )
    .build();
}

#[ktest]
#[should_panic]
fn dot_dot_class_device_name_panics_at_build() {
    // 1. Obtain the registered toy class.
    let (class, _) = register_toy_subsystems();

    // 2. Building the device must reject the name before registration.
    ClassDevice::builder(&class, "..", ToyDisk { sectors: 8 }).build();
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
    let (class, _) = register_toy_subsystems();

    // 2. Check that registration rejects each attribute and removes the directory.
    for attrs in INVALID_ATTRS {
        let dev = ClassDevice::builder(&class, "invalid-attr", ToyDisk { sectors: 8 })
            .attrs(attrs)
            .build();
        assert!(matches!(
            crate::add(&dev),
            Err(Error::SysTree(aster_systree::Error::InvalidName))
        ));
        assert!(lookup("/devices/virtual/toyblk/invalid-attr").is_none());
        assert!(lookup("/class/toyblk/invalid-attr").is_none());
    }
}
