// SPDX-License-Identifier: MPL-2.0

//! Kernel-mode tests with a synthetic bus, two device types, and two drivers.

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_systree::{SysBranchNode, SysObj, primary_tree};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{mm::VmWriter, prelude::ktest};

use crate::{
    AnyDevice, Attr, Bus, BusDevice, BusHandle, Class, ClassDevice, ClassHandle, ClassInterface,
    DevNode, DevNodeRequest, DevNum, DeviceType, Driver, Error, HookError, KernelHooks, Result,
    Uevent, UeventVars, add, init_for_ktest, register_bus, register_class, remove,
};

/// A fake bus: devices carry a vendor and a model id, drivers accept a vendor.
struct FakeBus;

struct FakeDev {
    vendor: u32,
    model: u32,
}

struct FakeMatch {
    vendor: u32,
}

const FAKE_BUS_ATTRS: &[Attr<BusDevice<FakeBus>>] = &[
    Attr::ro("vendor", |dev, w| {
        writeln!(w, "{:#06x}", dev.vendor)?;
        Ok(())
    }),
    Attr::ro("model", |dev, w| {
        writeln!(w, "{:#06x}", dev.model)?;
        Ok(())
    }),
];

impl Bus for FakeBus {
    const NAME: &'static str = "fake";
    type Device = FakeDev;
    type MatchData = FakeMatch;

    fn matches(&self, dev: &FakeDev, data: &FakeMatch) -> bool {
        dev.vendor == data.vendor
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        FAKE_BUS_ATTRS
    }

    fn uevent(&self, dev: &BusDevice<Self>, vars: &mut UeventVars) {
        vars.add(
            "MODALIAS",
            format_args!("fake:v{:08X}m{:08X}", dev.vendor, dev.model),
        );
    }
}

static DISK_TYPE: DeviceType<BusDevice<FakeBus>> = DeviceType::named("disk");

/// A driver that accepts vendor 1 and creates a class device on probe.
struct DiskDriver {
    match_data: FakeMatch,
    class: Arc<ClassHandle<FakeBlock>>,
    probes: AtomicUsize,
}

const DISK_DRIVER_ATTRS: &[Attr<BusDevice<FakeBus>>] = &[Attr::ro("bound_by", |_dev, w| {
    writeln!(w, "disk_driver")?;
    Ok(())
})];

impl Driver<FakeBus> for DiskDriver {
    fn name(&self) -> &str {
        "disk_driver"
    }

    fn match_data(&self) -> &FakeMatch {
        &self.match_data
    }

    fn probe(&self, dev: &Arc<BusDevice<FakeBus>>) -> Result<()> {
        self.probes.fetch_add(1, Ordering::Relaxed);
        let name = alloc::format!("fd{}", dev.model);
        let disk = ClassDevice::builder(&self.class, name, FakeBlockDev { sectors: 8 })
            .parent(dev.clone())
            .devnum(DevNum::block(DeviceId::new(
                MajorId::new(200),
                MinorId::new(dev.model),
            )))
            .build();
        add(&disk)?;
        Ok(())
    }

    fn remove(&self, dev: &Arc<BusDevice<FakeBus>>) {
        // The core removes the driver's files and links before calling this,
        // as Linux's `__device_release_driver` does.
        let path = dev.path().to_string();
        assert!(lookup(&alloc::format!("{path}/driver")).is_none());
        assert!(!attr_names(&path).iter().any(|name| name == "bound_by"));

        for child in dev.base().child_devices() {
            let _ = remove(&child.to_arc());
        }
    }

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<FakeBus>>] {
        DISK_DRIVER_ATTRS
    }
}

/// A driver that accepts vendor 2 and declines odd models.
struct PickyDriver {
    match_data: FakeMatch,
}

impl Driver<FakeBus> for PickyDriver {
    fn name(&self) -> &str {
        "picky"
    }

    fn match_data(&self) -> &FakeMatch {
        &self.match_data
    }

    fn probe(&self, dev: &Arc<BusDevice<FakeBus>>) -> Result<()> {
        if dev.model % 2 == 1 {
            return Err(Error::NoDriver);
        }
        Ok(())
    }
}

/// A fake block class.
struct FakeBlock;

struct FakeBlockDev {
    sectors: u64,
}

const FAKE_BLOCK_ATTRS: &[Attr<ClassDevice<FakeBlock>>] = &[Attr::ro("size", |dev, w| {
    writeln!(w, "{}", dev.sectors)?;
    Ok(())
})];

impl Class for FakeBlock {
    const NAME: &'static str = "fakeblock";
    type Device = FakeBlockDev;

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        FAKE_BLOCK_ATTRS
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

impl ClassInterface<FakeBlock> for CountingInterface {
    fn add_dev(&self, _dev: &Arc<ClassDevice<FakeBlock>>) {
        self.added.fetch_add(1, Ordering::Relaxed);
    }

    fn remove_dev(&self, _dev: &Arc<ClassDevice<FakeBlock>>) {
        self.removed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Resolves a path below the sysfs root, following no symlinks.
fn lookup(path: &str) -> Option<Arc<dyn SysObj>> {
    let mut node: Arc<dyn SysBranchNode> = primary_tree().root().clone();
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

fn link_target(path: &str) -> Option<String> {
    lookup(path)?
        .cast_to_symlink()
        .map(|l| l.target_path().to_string())
}

fn read_attr(path: &str, name: &str) -> String {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    let mut buf = [0u8; 512];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let len = node.read_attr(name, &mut writer).unwrap();
    String::from_utf8_lossy(&buf[..len]).to_string()
}

fn attr_ids(path: &str) -> Vec<(String, u8)> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .to_vec()
        .into_iter()
        .map(|a| (a.name().to_string(), a.id()))
        .collect()
}

fn attr_names(path: &str) -> Vec<String> {
    let node = lookup(path).unwrap().cast_to_node().unwrap();
    node.node_attrs()
        .to_vec()
        .into_iter()
        .map(|a| a.name().to_string())
        .collect()
}

struct Fixture {
    bus: Arc<BusHandle<FakeBus>>,
    class: Arc<ClassHandle<FakeBlock>>,
}

static FIXTURE: spin::Once<Fixture> = spin::Once::new();

fn fixture() -> &'static Fixture {
    init_for_ktest();
    FIXTURE.call_once(|| {
        let bus = register_bus(FakeBus).unwrap();
        let class = register_class(FakeBlock).unwrap();
        Fixture { bus, class }
    })
}

#[ktest]
fn roots_exist() {
    fixture();
    for path in [
        "/devices",
        "/devices/virtual",
        "/bus",
        "/class",
        "/dev/char",
        "/dev/block",
        "/bus/fake/devices",
        "/bus/fake/drivers",
        "/class/fakeblock",
    ] {
        assert!(lookup(path).is_some(), "{path} missing");
    }
    assert_eq!(read_attr("/bus/fake", "drivers_autoprobe"), "1\n");
}

#[ktest]
fn bus_device_registration_and_probe() {
    let fx = fixture();
    let driver = fx
        .bus
        .register_driver(Arc::new(DiskDriver {
            match_data: FakeMatch { vendor: 1 },
            class: fx.class.clone(),
            probes: AtomicUsize::new(0),
        }))
        .unwrap();
    let iface = Arc::new(CountingInterface {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    fx.class
        .register_interface(iface.clone() as Arc<dyn ClassInterface<FakeBlock>>);

    let root = crate::BareDevice::new_root("fake0000:00");
    add(&root).unwrap();
    let dev = BusDevice::builder(
        &fx.bus,
        "0000:00:01.0",
        FakeDev {
            vendor: 1,
            model: 3,
        },
    )
    .parent(root.clone())
    .dev_type(&DISK_TYPE)
    .build();
    add(&dev).unwrap();

    // Placement and links of the bus device.
    assert_eq!(dev.path(), "/devices/fake0000:00/0000:00:01.0");
    assert_eq!(
        link_target("/devices/fake0000:00/0000:00:01.0/subsystem").unwrap(),
        "../../../bus/fake"
    );
    assert_eq!(
        link_target("/bus/fake/devices/0000:00:01.0").unwrap(),
        "../../../devices/fake0000:00/0000:00:01.0"
    );
    // Binding.
    assert!(dev.driver().is_some_and(|d| Arc::ptr_eq(&d, &driver)));
    assert_eq!(
        link_target("/devices/fake0000:00/0000:00:01.0/driver").unwrap(),
        "../../../bus/fake/drivers/disk_driver"
    );
    assert_eq!(
        link_target("/bus/fake/drivers/disk_driver/0000:00:01.0").unwrap(),
        "../../../../devices/fake0000:00/0000:00:01.0"
    );
    // Attributes from the bus, the driver, and the core.
    let names = attr_names("/devices/fake0000:00/0000:00:01.0");
    for expected in ["uevent", "vendor", "model", "bound_by"] {
        assert!(names.iter().any(|n| n == expected), "{expected} missing");
    }
    assert_eq!(
        read_attr("/devices/fake0000:00/0000:00:01.0", "vendor"),
        "0x0001\n"
    );
    assert_eq!(
        read_attr("/devices/fake0000:00/0000:00:01.0", "uevent"),
        "DEVTYPE=disk\nDRIVER=disk_driver\nMODALIAS=fake:v00000001m00000003\n"
    );

    // The class device the driver created, in a glue directory.
    let disk_path = "/devices/fake0000:00/0000:00:01.0/fakeblock/fd3";
    assert!(lookup(disk_path).is_some(), "class device missing");
    assert_eq!(
        link_target(&alloc::format!("{disk_path}/device")).unwrap(),
        "../../../0000:00:01.0"
    );
    assert_eq!(
        link_target(&alloc::format!("{disk_path}/subsystem")).unwrap(),
        "../../../../../class/fakeblock"
    );
    assert_eq!(
        link_target("/class/fakeblock/fd3").unwrap(),
        "../../devices/fake0000:00/0000:00:01.0/fakeblock/fd3"
    );
    assert_eq!(
        link_target("/dev/block/200:3").unwrap(),
        "../../devices/fake0000:00/0000:00:01.0/fakeblock/fd3"
    );
    assert_eq!(read_attr(disk_path, "dev"), "200:3\n");
    assert_eq!(read_attr(disk_path, "size"), "8\n");
    assert_eq!(
        read_attr(disk_path, "uevent"),
        "MAJOR=200\nMINOR=3\nDEVNAME=fd3\nDEVMODE=0660\n"
    );
    assert_eq!(iface.added.load(Ordering::Relaxed), 1);

    // Unbind: the driver removes its class device, driver attributes and
    // links go away, the glue directory disappears once empty.
    fx.bus.unbind(&dev).unwrap();
    assert!(dev.driver().is_none());
    assert!(lookup("/devices/fake0000:00/0000:00:01.0/driver").is_none());
    assert!(lookup("/bus/fake/drivers/disk_driver/0000:00:01.0").is_none());
    assert!(
        !attr_names("/devices/fake0000:00/0000:00:01.0")
            .iter()
            .any(|n| n == "bound_by")
    );
    assert!(lookup(disk_path).is_none());
    assert!(lookup("/devices/fake0000:00/0000:00:01.0/fakeblock").is_none());
    assert!(lookup("/class/fakeblock/fd3").is_none());
    assert!(lookup("/dev/block/200:3").is_none());
    assert_eq!(iface.removed.load(Ordering::Relaxed), 1);

    // Rebind through the sysfs control file, then remove everything.
    let driver_dir = lookup("/bus/fake/drivers/disk_driver")
        .unwrap()
        .cast_to_node()
        .unwrap();
    driver_dir.store_attr("bind", "0000:00:01.0\n").unwrap();
    assert!(dev.driver().is_some());
    assert!(lookup(disk_path).is_some());

    // A device with children cannot be removed.
    assert!(matches!(remove(&dev), Err(Error::HasChildren)));
    fx.bus.unbind(&dev).unwrap();
    remove(&dev).unwrap();
    assert!(lookup("/devices/fake0000:00/0000:00:01.0").is_none());
    assert!(lookup("/bus/fake/devices/0000:00:01.0").is_none());
    assert!(matches!(remove(&dev), Err(Error::NotAdded)));
    assert!(matches!(add(&dev), Err(Error::AlreadyAdded)));
    remove(&root).unwrap();
    fx.bus.unregister_driver(&driver).unwrap();
    fx.class
        .unregister_interface(&(iface as Arc<dyn ClassInterface<FakeBlock>>))
        .unwrap();
}

#[ktest]
fn driver_registered_after_device_and_probe_failure() {
    let fx = fixture();
    let even = BusDevice::builder(
        &fx.bus,
        "picky-even",
        FakeDev {
            vendor: 2,
            model: 4,
        },
    )
    .build();
    let odd = BusDevice::builder(
        &fx.bus,
        "picky-odd",
        FakeDev {
            vendor: 2,
            model: 5,
        },
    )
    .build();
    add(&even).unwrap();
    add(&odd).unwrap();
    // Parentless bus devices sit at the top of /sys/devices.
    assert_eq!(even.path(), "/devices/picky-even");
    assert!(even.driver().is_none());

    let picky = fx
        .bus
        .register_driver(Arc::new(PickyDriver {
            match_data: FakeMatch { vendor: 2 },
        }))
        .unwrap();
    // The driver arrived after the devices and bound the one it accepts.
    assert!(even.driver().is_some());
    assert!(odd.driver().is_none());
    assert_eq!(picky.devices().len(), 1);
    assert!(matches!(
        fx.bus.bind(&odd, &picky),
        Err(Error::ProbeFailed | Error::NoDriver)
    ));

    fx.bus.unregister_driver(&picky).unwrap();
    assert!(even.driver().is_none());
    assert!(lookup("/bus/fake/drivers/picky").is_none());
    remove(&even).unwrap();
    remove(&odd).unwrap();
}

#[ktest]
fn control_files_bind_probe_and_unbind() {
    let fx = fixture();
    let disk = fx
        .bus
        .register_driver(Arc::new(DiskDriver {
            match_data: FakeMatch { vendor: 1 },
            class: fx.class.clone(),
            probes: AtomicUsize::new(0),
        }))
        .unwrap();
    let picky = fx
        .bus
        .register_driver(Arc::new(PickyDriver {
            match_data: FakeMatch { vendor: 2 },
        }))
        .unwrap();

    let dev = BusDevice::builder(
        &fx.bus,
        "ctl0",
        FakeDev {
            vendor: 2,
            model: 4,
        },
    )
    .build();
    add(&dev).unwrap();
    assert!(dev.driver().is_some_and(|d| Arc::ptr_eq(&d, &picky)));

    let bus_dir = lookup("/bus/fake").unwrap().cast_to_node().unwrap();
    let disk_dir = lookup("/bus/fake/drivers/disk_driver")
        .unwrap()
        .cast_to_node()
        .unwrap();
    let picky_dir = lookup("/bus/fake/drivers/picky")
        .unwrap()
        .cast_to_node()
        .unwrap();

    // Probing a device that is already bound is not an error, as in Linux.
    bus_dir.store_attr("drivers_probe", "ctl0\n").unwrap();
    assert!(dev.driver().is_some());
    // Nor is probing a device no driver accepts.
    let orphan = BusDevice::builder(
        &fx.bus,
        "ctl1",
        FakeDev {
            vendor: 9,
            model: 0,
        },
    )
    .build();
    add(&orphan).unwrap();
    bus_dir.store_attr("drivers_probe", "ctl1\n").unwrap();
    assert!(orphan.driver().is_none());
    // An unknown device name is.
    assert!(bus_dir.store_attr("drivers_probe", "nosuchdev\n").is_err());

    // Unbinding through the wrong driver's file leaves the binding alone.
    assert!(disk_dir.store_attr("unbind", "ctl0\n").is_err());
    assert!(dev.driver().is_some_and(|d| Arc::ptr_eq(&d, &picky)));
    // Through the right one it works.
    picky_dir.store_attr("unbind", "ctl0\n").unwrap();
    assert!(dev.driver().is_none());

    remove(&orphan).unwrap();
    remove(&dev).unwrap();
    fx.bus.unregister_driver(&picky).unwrap();
    fx.bus.unregister_driver(&disk).unwrap();
}

#[ktest]
fn parentless_class_device_lives_under_virtual() {
    let fx = fixture();
    let dev = ClassDevice::builder(&fx.class, "loop0", FakeBlockDev { sectors: 1 })
        .devnum(DevNum::block(DeviceId::new(
            MajorId::new(7),
            MinorId::new(0),
        )))
        .build();
    add(&dev).unwrap();
    assert_eq!(dev.path(), "/devices/virtual/fakeblock/loop0");
    assert_eq!(
        link_target("/class/fakeblock/loop0").unwrap(),
        "../../devices/virtual/fakeblock/loop0"
    );
    assert_eq!(
        link_target("/devices/virtual/fakeblock/loop0/subsystem").unwrap(),
        "../../../../class/fakeblock"
    );
    assert!(lookup("/devices/virtual/fakeblock/loop0/device").is_none());
    remove(&dev).unwrap();
    assert!(lookup("/devices/virtual/fakeblock").is_none());
}

#[ktest]
fn class_device_under_class_device_has_no_glue_dir() {
    let fx = fixture();
    let disk = ClassDevice::builder(&fx.class, "sd0", FakeBlockDev { sectors: 16 }).build();
    add(&disk).unwrap();
    let part = ClassDevice::builder(&fx.class, "sd0p1", FakeBlockDev { sectors: 8 })
        .parent(disk.clone())
        .build();
    add(&part).unwrap();
    assert_eq!(part.path(), "/devices/virtual/fakeblock/sd0/sd0p1");
    remove(&part).unwrap();
    remove(&disk).unwrap();
}

#[ktest]
fn invalid_states_are_rejected() {
    let fx = fixture();
    let bad = ClassDevice::builder(&fx.class, "a/b", FakeBlockDev { sectors: 0 }).build();
    assert!(matches!(add(&bad), Err(Error::InvalidName)));

    let parent =
        ClassDevice::builder(&fx.class, "orphan-parent", FakeBlockDev { sectors: 0 }).build();
    let child = ClassDevice::builder(&fx.class, "orphan", FakeBlockDev { sectors: 0 })
        .parent(parent.clone())
        .build();
    assert!(matches!(add(&child), Err(Error::ParentNotAdded)));

    let a = ClassDevice::builder(&fx.class, "twin", FakeBlockDev { sectors: 0 }).build();
    let b = ClassDevice::builder(&fx.class, "twin", FakeBlockDev { sectors: 0 }).build();
    add(&a).unwrap();
    assert!(matches!(add(&b), Err(Error::NameConflict)));
    // The failed registration took nothing of `a` with it.
    assert!(lookup("/devices/virtual/fakeblock/twin").is_some());
    assert_eq!(
        link_target("/class/fakeblock/twin").unwrap(),
        "../../devices/virtual/fakeblock/twin"
    );
    remove(&a).unwrap();
    assert!(lookup("/class/fakeblock/twin").is_none());

    // Two devices of one name under different parents: the second fails on
    // the index link, and the first keeps its index entry.
    let p1 = ClassDevice::builder(&fx.class, "p1", FakeBlockDev { sectors: 0 }).build();
    let p2 = ClassDevice::builder(&fx.class, "p2", FakeBlockDev { sectors: 0 }).build();
    add(&p1).unwrap();
    add(&p2).unwrap();
    let c1 = ClassDevice::builder(&fx.class, "same", FakeBlockDev { sectors: 0 })
        .parent(p1.clone())
        .build();
    let c2 = ClassDevice::builder(&fx.class, "same", FakeBlockDev { sectors: 0 })
        .parent(p2.clone())
        .build();
    add(&c1).unwrap();
    assert!(matches!(add(&c2), Err(Error::NameConflict)));
    assert_eq!(
        link_target("/class/fakeblock/same").unwrap(),
        "../../devices/virtual/fakeblock/p1/same"
    );
    assert!(lookup("/devices/virtual/fakeblock/p2/same").is_none());
    remove(&c1).unwrap();
    remove(&p2).unwrap();
    remove(&p1).unwrap();
}

struct RecordingHooks {
    created: ostd::sync::Mutex<Vec<String>>,
}

impl KernelHooks for RecordingHooks {
    fn create_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError> {
        self.created.lock().push(request.path.to_string());
        Ok(())
    }

    fn delete_devnode(&self, _request: &DevNodeRequest) -> core::result::Result<(), HookError> {
        Ok(())
    }

    fn broadcast_uevent(&self, _event: &Uevent) {}
}

#[ktest]
fn requests_queued_before_hooks_are_replayed() {
    // A slot of its own, so the test does not depend on whether the kernel
    // has already installed its hooks into the global one.
    let slot = crate::hooks::HookSlot::new();
    let request = DevNodeRequest {
        devnum: DevNum::block(DeviceId::new(MajorId::new(7), MinorId::new(9))),
        path: crate::SysStr::from("queued0"),
        mode: 0o660,
    };
    // With no hooks installed the request is accepted and queued.
    slot.create_devnode(request.clone()).unwrap();

    let hooks = Arc::new(RecordingHooks {
        created: ostd::sync::Mutex::new(Vec::new()),
    });
    slot.install(hooks.clone());
    assert_eq!(hooks.created.lock().as_slice(), ["queued0".to_string()]);

    // Once installed, a request reaches the hooks directly.
    slot.create_devnode(request).unwrap();
    assert_eq!(hooks.created.lock().len(), 2);
}

#[ktest]
fn failed_add_does_not_notify_interfaces() {
    let fx = fixture();
    let iface = Arc::new(CountingInterface {
        added: AtomicUsize::new(0),
        removed: AtomicUsize::new(0),
    });
    fx.class
        .register_interface(iface.clone() as Arc<dyn ClassInterface<FakeBlock>>);
    let before_added = iface.added.load(Ordering::Relaxed);

    let a = ClassDevice::builder(&fx.class, "clash", FakeBlockDev { sectors: 0 }).build();
    let b = ClassDevice::builder(&fx.class, "clash", FakeBlockDev { sectors: 0 }).build();
    add(&a).unwrap();
    assert!(matches!(add(&b), Err(Error::NameConflict)));
    assert_eq!(iface.added.load(Ordering::Relaxed), before_added + 1);
    assert_eq!(iface.removed.load(Ordering::Relaxed), 0);
    remove(&a).unwrap();
    assert_eq!(iface.removed.load(Ordering::Relaxed), 1);
    fx.class
        .unregister_interface(&(iface as Arc<dyn ClassInterface<FakeBlock>>))
        .unwrap();
}

/// A driver's attributes come and go, and the published attribute set is
/// rebuilt each time. The attributes that survive must keep their IDs, because
/// sysfs derives an inode number from `(node id, attribute id)`: renumbering
/// them would give every file in a device's directory a new inode whenever a
/// driver bound beside it.
#[ktest]
fn attribute_ids_survive_bind_and_unbind() {
    let fx = fixture();

    // Vendor 9 matches no driver registered by another test, so the device
    // starts out unbound and carries only its core, bus and own attributes.
    let dev = BusDevice::builder(
        &fx.bus,
        "ids0",
        FakeDev {
            vendor: 9,
            model: 91,
        },
    )
    .build();
    add(&dev).unwrap();
    let path = "/devices/ids0";
    let before = attr_ids(path);
    assert!(!before.is_empty());
    assert!(before.iter().all(|(name, _)| name != "bound_by"));

    // Registering a matching driver binds it, which adds `bound_by`.
    let driver = fx
        .bus
        .register_driver(Arc::new(DiskDriver {
            match_data: FakeMatch { vendor: 9 },
            class: fx.class.clone(),
            probes: AtomicUsize::new(0),
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

    // Unbinding takes it away and leaves the rest exactly as they were.
    fx.bus.unbind(&dev).unwrap();
    assert_eq!(attr_ids(path), before);

    // The ID it used is reusable rather than leaked, so repeated binds cannot
    // exhaust the 8-bit ID space.
    fx.bus.bind(&dev, &driver).unwrap();
    assert_eq!(
        attr_ids(path)
            .iter()
            .find(|(n, _)| n == "bound_by")
            .map(|(_, i)| *i),
        Some(bound_by_id)
    );

    fx.bus.unbind(&dev).unwrap();
    remove(&dev).unwrap();
    fx.bus.unregister_driver(&driver).unwrap();
}

/// Registration hands the core, subsystem, type and own attributes over as one
/// batch, so a layer that reuses a name the core already took must be rejected
/// there. Otherwise its callback would quietly replace the core's while sysfs
/// went on showing the core's entry.
#[ktest]
fn an_attribute_may_not_shadow_a_core_one() {
    const SHADOWING: &[Attr<BusDevice<FakeBus>>] = &[Attr::ro("uevent", |_dev, w| {
        writeln!(w, "not the core's uevent")?;
        Ok(())
    })];

    let fx = fixture();
    let dev = BusDevice::builder(
        &fx.bus,
        "shadow0",
        FakeDev {
            vendor: 8,
            model: 1,
        },
    )
    .attrs(SHADOWING)
    .build();

    assert!(matches!(add(&dev), Err(Error::NameConflict)));
    // The failed registration is undone: no directory is left behind.
    assert!(lookup("/devices/shadow0").is_none());
}
