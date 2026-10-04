// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use aster_systree::{SysNode, SysObj};
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{prelude::ktest, sync::Mutex};

use super::utils;
use crate::{
    bus::{self, Bus, BusDevice, BusHandle, Driver},
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, BareDevice, DevNode, DevNum, DeviceType, Error, Result},
    uevent::{self, UeventAction, UeventVars},
};

#[ktest]
fn sysfs_reports_frozen_devnode_policy_and_ordered_callbacks() {
    let (class, _) = subsystems();
    let dev = ClassDevice::builder(&class, "env!node", NodePayload::new())
        .devnum(DevNum::char(DeviceId::new(
            MajorId::new(240),
            MinorId::new(1),
        )))
        .dev_type(&NODE_TYPE)
        .build();
    crate::add_device(&dev).unwrap();
    assert_eq!(dev.path(), "/devices/virtual/uevent-test-class/env!node");
    dev.change_node_name.store(true, Ordering::Relaxed);

    let before = uevent::current_seqnum();
    let environment = dev.show_attr("uevent").unwrap();
    for entry in [
        "MAJOR=240",
        "MINOR=1",
        "DEVNAME=events/original",
        "DEVMODE=0660",
        "DEVTYPE=event-node",
    ] {
        assert!(environment.lines().any(|line| line == entry), "{entry}");
    }
    assert_eq!(
        environment
            .lines()
            .filter(|line| line.starts_with("SOURCE="))
            .collect::<Vec<_>>(),
        vec!["SOURCE=class", "SOURCE=type"]
    );
    assert_eq!(
        uevent::current_seqnum(),
        before,
        "reading must not allocate a sequence number"
    );
    crate::remove_device(&dev).unwrap();
    assert!(matches!(
        dev.show_attr("uevent"),
        Err(aster_systree::Error::IsDead)
    ));
}
#[ktest]
fn sysfs_distinguishes_absent_zero_and_explicit_node_modes() {
    let (class, _) = subsystems();
    for (mode, expected) in [
        (None, None),
        (Some(0), None),
        (Some(0o600), Some("0600")),
        (Some(0o1660), Some("0660")),
        (Some(0o1000), Some("0")),
    ] {
        let mut payload = NodePayload::new();
        payload.mode = mode;
        let dev = ClassDevice::builder(&class, "mode-policy", payload)
            .devnum(DevNum::char(DeviceId::new(
                MajorId::new(240),
                MinorId::new(2),
            )))
            .build();
        crate::add_device(&dev).unwrap();
        let environment = dev.show_attr("uevent").unwrap();
        assert_eq!(
            environment
                .lines()
                .find_map(|line| line.strip_prefix("DEVMODE=")),
            expected,
            "unexpected event permissions for {mode:?}"
        );
        crate::remove_device(&dev).unwrap();
    }
}

#[ktest]
fn synthetic_sysfs_writes_preserve_membership_and_map_errors() {
    let (class, _) = subsystems();
    let dev = ClassDevice::builder(&class, "synthetic", NodePayload::new()).build();
    crate::add_device(&dev).unwrap();
    dev.store_attr("uevent", "remove").unwrap();
    let uuid = "12345678-ABCD-1234-abcd-123456789abc";
    assert!(
        class.find_device("synthetic").is_some(),
        "synthetic remove must not unregister the device"
    );
    assert!(matches!(
        dev.store_attr("uevent", "add 12345678-1234-1234-1234-123456789abé"),
        Err(aster_systree::Error::InvalidOperation)
    ));

    let mut request = alloc::format!("change {uuid}");
    for index in 0..60 {
        request.push_str(&alloc::format!(" K{index}=v"));
    }
    assert!(matches!(
        dev.store_attr("uevent", &request),
        Err(aster_systree::Error::NoMemory)
    ));
    request.push_str(" K60=v K61=v K62=v K63=v");
    assert!(matches!(
        dev.store_attr("uevent", &request),
        Err(aster_systree::Error::InvalidOperation)
    ));
    crate::remove_device(&dev).unwrap();
    assert!(matches!(
        dev.store_attr("uevent", "change"),
        Err(aster_systree::Error::IsDead)
    ));
}

#[ktest]
fn lifecycle_callbacks_observe_registration_and_driver_binding() {
    let (_, bus) = subsystems();
    let root = BareDevice::new_root("uevent-root");
    crate::add_device(&root).unwrap();
    let driver = bus.register_driver(Arc::new(EventDriver)).unwrap();
    let dev = BusDevice::builder(&bus, "lifecycle", Mutex::new(Vec::new()))
        .parent(root.clone())
        .devnum(DevNum::char(DeviceId::new(
            MajorId::new(240),
            MinorId::new(3),
        )))
        .dev_type(&BUS_TYPE)
        .build();
    crate::add_device(&dev).unwrap();
    assert_eq!(
        *dev.payload().lock(),
        vec![
            (None, "/devices/uevent-root/lifecycle".to_string()),
            (
                Some("event-driver".to_string()),
                "/devices/uevent-root/lifecycle".to_string()
            )
        ]
    );
    assert!(
        dev.show_attr("uevent")
            .unwrap()
            .contains("DRIVER=event-driver\n")
    );
    dev.payload().lock().clear();
    bus.unbind(&dev).unwrap();
    assert_eq!(
        *dev.payload().lock(),
        vec![(None, "/devices/uevent-root/lifecycle".to_string())]
    );
    let driver_dir = utils::lookup("/bus/uevent-test-bus/drivers/event-driver")
        .unwrap()
        .cast_to_node()
        .unwrap();
    driver_dir.store_attr("bind", "lifecycle\n").unwrap();
    dev.payload().lock().clear();
    crate::remove_device(&dev).unwrap();
    assert!(
        crate::emit_uevent(&dev, UeventAction::Change).is_ok(),
        "a post-remove change keeps the frozen devnode policy"
    );
    crate::remove_device(&root).unwrap();
    bus.unregister_driver(&driver).unwrap();
}

#[ktest]
fn automatic_binding_events_allow_driver_attribute_reads_and_rebinding() {
    static REBIND_ON_UNBOUND: AtomicBool = AtomicBool::new(false);
    static ATTRIBUTE_READS: AtomicUsize = AtomicUsize::new(0);
    static REENTRANT_TYPE: DeviceType<BusDevice<EventBus>> = {
        let mut dev_type = DeviceType::named("binding-reentry");
        dev_type.uevent_fn = Some(|dev: &BusDevice<EventBus>, _vars| {
            if dev.driver().is_some() {
                assert_eq!(dev.show_attr("bound_device").unwrap(), "binding-reentry\n");
                ATTRIBUTE_READS.fetch_add(1, Ordering::Relaxed);
            } else if REBIND_ON_UNBOUND.swap(false, Ordering::Relaxed) {
                utils::lookup("/bus/uevent-test-bus/drivers/reentrant-event-driver")
                    .unwrap()
                    .cast_to_node()
                    .unwrap()
                    .store_attr("bind", dev.name())
                    .unwrap();
            }
            Ok(())
        });
        dev_type
    };
    struct ReentrantDriver;
    impl Driver<EventBus> for ReentrantDriver {
        fn name(&self) -> &str {
            "reentrant-event-driver"
        }
        fn match_data(&self) -> &() {
            &()
        }
        fn on_probe(&self, _dev: &Arc<BusDevice<EventBus>>) -> Result<()> {
            Ok(())
        }
        fn dev_attrs(&self) -> &'static [Attr<BusDevice<EventBus>>] {
            const ATTRS: &[Attr<BusDevice<EventBus>>] =
                &[Attr::ro("bound_device", |dev, writer| {
                    writeln!(writer, "{}", dev.name())?;
                    Ok(())
                })];
            ATTRS
        }
    }

    let (_, bus) = subsystems();
    REBIND_ON_UNBOUND.store(false, Ordering::Relaxed);
    ATTRIBUTE_READS.store(0, Ordering::Relaxed);
    let driver = bus.register_driver(Arc::new(ReentrantDriver)).unwrap();
    let dev = BusDevice::builder(&bus, "binding-reentry", Mutex::new(Vec::new()))
        .dev_type(&REENTRANT_TYPE)
        .build();
    crate::add_device(&dev).unwrap();
    assert_eq!(ATTRIBUTE_READS.load(Ordering::Relaxed), 1);
    REBIND_ON_UNBOUND.store(true, Ordering::Relaxed);
    bus.unbind(&dev).unwrap();
    assert!(!REBIND_ON_UNBOUND.load(Ordering::Relaxed));
    assert_eq!(ATTRIBUTE_READS.load(Ordering::Relaxed), 2);
    assert!(
        dev.driver()
            .is_some_and(|bound| Arc::ptr_eq(&bound, &driver))
    );

    crate::remove_device(&dev).unwrap();
    assert!(dev.driver().is_none());
    assert!(utils::lookup("/devices/binding-reentry").is_none());
    bus.unregister_driver(&driver).unwrap();
}

#[ktest]
fn callback_failure_is_best_effort_only_for_lifecycle_events() {
    let (class, _) = subsystems();
    let dev = ClassDevice::builder(&class, "callback-failure", NodePayload::new()).build();
    dev.fail_event.store(true, Ordering::Relaxed);
    crate::add_device(&dev).unwrap();
    assert!(class.find_device("callback-failure").is_some());
    assert!(matches!(
        crate::emit_uevent(&dev, UeventAction::Change),
        Err(Error::Attribute)
    ));
    assert!(matches!(
        dev.store_attr("uevent", "change"),
        Err(aster_systree::Error::AttributeError)
    ));
    assert!(matches!(
        dev.show_attr("uevent"),
        Err(aster_systree::Error::AttributeError)
    ));
    crate::remove_device(&dev).unwrap();
    assert!(class.find_device("callback-failure").is_none());
}

#[ktest]
fn registration_rollback_does_not_announce_add_or_remove() {
    let (class, _) = subsystems();
    let first = ClassDevice::builder(&class, "rollback-event", NodePayload::new()).build();
    let duplicate = ClassDevice::builder(&class, "rollback-event", NodePayload::new()).build();
    crate::add_device(&first).unwrap();
    let before = uevent::current_seqnum();
    assert!(matches!(
        crate::add_device(&duplicate),
        Err(Error::NameConflict)
    ));
    assert_eq!(uevent::current_seqnum(), before);
    assert!(
        class
            .find_device("rollback-event")
            .is_some_and(|dev| Arc::ptr_eq(&dev, &first))
    );
    crate::remove_device(&first).unwrap();
}

type EventSubsystems = (Arc<ClassHandle<EventClass>>, Arc<BusHandle<EventBus>>);

fn subsystems() -> EventSubsystems {
    static SUBSYSTEMS: spin::Once<EventSubsystems> = spin::Once::new();
    crate::init_for_ktest();
    SUBSYSTEMS
        .call_once(|| {
            (
                class::register(EventClass).unwrap(),
                bus::register(EventBus).unwrap(),
            )
        })
        .clone()
}

struct NodePayload {
    change_node_name: AtomicBool,
    fail_event: AtomicBool,
    mode: Option<u16>,
}

impl NodePayload {
    fn new() -> Self {
        Self {
            change_node_name: AtomicBool::new(false),
            fail_event: AtomicBool::new(false),
            mode: Some(0o1660),
        }
    }
}

struct EventClass;

impl Class for EventClass {
    const NAME: &'static str = "uevent-test-class";
    type Device = NodePayload;
    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }
    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        Some(DevNode {
            path: Some(
                if dev.change_node_name.load(Ordering::Relaxed) {
                    "events/changed"
                } else {
                    "events/original"
                }
                .into(),
            ),
            mode: dev.mode,
        })
    }
    fn uevent(&self, dev: &ClassDevice<Self>, vars: &mut UeventVars) -> Result<()> {
        if dev.fail_event.load(Ordering::Relaxed) {
            return Err(Error::Attribute);
        }
        vars.add("SOURCE", "class")
    }
}

static NODE_TYPE: DeviceType<ClassDevice<EventClass>> = {
    let mut dev_type = DeviceType::named("event-node");
    dev_type.uevent_fn = Some(|_dev, vars| vars.add("SOURCE", "type"));
    dev_type
};

struct EventBus;

impl Bus for EventBus {
    const NAME: &'static str = "uevent-test-bus";
    type Device = Mutex<Vec<(Option<String>, String)>>;
    type MatchData = ();
    fn matches(&self, _dev: &Self::Device, _data: &Self::MatchData) -> bool {
        true
    }
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        &[]
    }
    fn uevent(&self, dev: &BusDevice<Self>, vars: &mut UeventVars) -> Result<()> {
        assert!(matches!(
            crate::add_device(&dev.to_arc()),
            Err(Error::AlreadyAdded)
        ));
        dev.payload().lock().push((
            dev.driver().map(|driver| driver.name().to_string()),
            dev.path().to_string(),
        ));
        vars.add("SOURCE", "bus")?;
        vars.add("MODALIAS", "uevent-test")
    }
}

static BUS_TYPE: DeviceType<BusDevice<EventBus>> = DeviceType::named("event-bus-device");
struct EventDriver;

impl Driver<EventBus> for EventDriver {
    fn name(&self) -> &str {
        "event-driver"
    }
    fn match_data(&self) -> &() {
        &()
    }
    fn on_probe(&self, _dev: &Arc<BusDevice<EventBus>>) -> Result<()> {
        Ok(())
    }
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<EventBus>>] {
        &[]
    }
}
