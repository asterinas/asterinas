// SPDX-License-Identifier: MPL-2.0

use alloc::{
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_systree::SysNode;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::{prelude::ktest, sync::Mutex};

use super::utils;
use crate::{
    bus::{self, Bus, BusDevice, BusHandle, Driver},
    class::{self, Class, ClassDevice, ClassHandle},
    common::{Attr, DevNode, DevNum, DeviceType, Error, Result},
    uevent::{self, UeventAction, UeventBuilder, UeventVars},
};

#[ktest]
fn synthetic_environment_preserves_uuid_and_duplicate_argument_order() {
    let (action, vars) =
        uevent::parse_synthetic("change 12345678-ABCD-1234-abcd-123456789abc FOO=1 BAR=2 FOO=3\0")
            .unwrap();
    assert_eq!(action, UeventAction::Change);
    assert_eq!(
        vars.iter().collect::<Vec<_>>(),
        vec![
            ("SYNTH_UUID", "12345678-ABCD-1234-abcd-123456789abc"),
            ("SYNTH_ARG_FOO", "1"),
            ("SYNTH_ARG_BAR", "2"),
            ("SYNTH_ARG_FOO", "3"),
        ]
    );
    for (request, expected) in [
        ("add", UeventAction::Add),
        ("remove", UeventAction::Remove),
        ("change", UeventAction::Change),
        ("move", UeventAction::Move),
        ("online", UeventAction::Online),
        ("offline", UeventAction::Offline),
        ("bind", UeventAction::Bind),
        ("unbind", UeventAction::Unbind),
    ] {
        let (action, vars) = uevent::parse_synthetic(&alloc::format!("{request}\n")).unwrap();
        assert_eq!(action, expected);
        assert_eq!(vars.iter().collect::<Vec<_>>(), vec![("SYNTH_UUID", "0")]);
    }
}

#[ktest]
fn synthetic_requests_reject_malformed_actions_uuids_and_arguments() {
    for request in [
        "",
        "Change",
        "unknown",
        " change",
        "change ",
        "change\n\n",
        "change\0\n",
        "change\t",
        "change\r",
        "change  12345678-1234-1234-1234-123456789abc",
        "change 12345678-1234-1234-1234-123456789ab",
        "change 12345678-1234-1234-1234-123456789abcc",
        "change 12345678_1234-1234-1234-123456789abc",
        "change 12345678-1234-1234-1234-123456789abg",
        "change 12345678-1234-1234-1234-123456789abé",
        "change FOO=1",
        "change 12345678-1234-1234-1234-123456789abc FOO",
        "change 12345678-1234-1234-1234-123456789abc =1",
        "change 12345678-1234-1234-1234-123456789abc FOO=",
        "change 12345678-1234-1234-1234-123456789abc F_OO=1",
        "change 12345678-1234-1234-1234-123456789abc FOO=1=2",
        "change 12345678-1234-1234-1234-123456789abc FOO=é",
        "change 12345678-1234-1234-1234-123456789abc FOO=1 ",
    ] {
        assert!(
            matches!(uevent::parse_synthetic(request), Err(Error::InvalidValue)),
            "{request:?}"
        );
    }
}

#[ktest]
fn synthetic_environment_overflow_maps_to_invalid_value() {
    let mut request = String::from("change 12345678-1234-1234-1234-123456789abc");
    for _ in 0..63 {
        request.push_str(" K=v");
    }
    assert_eq!(
        uevent::parse_synthetic(&request).unwrap().1.iter().count(),
        64
    );
    request.push_str(" K=v");
    assert!(matches!(
        uevent::parse_synthetic(&request),
        Err(Error::InvalidValue)
    ));

    let prefix = "change 12345678-1234-1234-1234-123456789abc K=";
    let value_len = 2048 - "SYNTH_UUID=12345678-1234-1234-1234-123456789abc\0SYNTH_ARG_K=\0".len();
    let mut request = alloc::format!("{prefix}{}", "v".repeat(value_len));
    assert_eq!(
        uevent::parse_synthetic(&request).unwrap().1.total_bytes(),
        2048
    );
    request.push('v');
    assert!(matches!(
        uevent::parse_synthetic(&request),
        Err(Error::InvalidValue)
    ));
}

#[ktest]
fn variable_validation_does_not_modify_an_existing_environment() {
    let mut vars = UeventVars::new();
    vars.add("KEEP", "value").unwrap();
    let expected = vars.clone();
    for (key, value) in [("", "x"), ("K=V", "x"), ("K\0", "x"), ("K", "x\0y")] {
        assert!(matches!(vars.add(key, value), Err(Error::InvalidValue)));
        assert_eq!(vars, expected);
    }
    assert!(matches!(
        vars.add("BIG", "v".repeat(2048)),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(vars, expected);
}

#[ktest]
fn environment_limits_preserve_failed_append_state() {
    let mut vars = UeventVars::new();
    vars.add("K", "v".repeat(2045)).unwrap();
    assert_eq!(vars.total_bytes(), 2048);
    let snapshot = vars.clone();
    assert!(matches!(
        vars.add("NEXT", "v"),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(vars, snapshot);

    let mut vars = UeventVars::new();
    for _ in 0..64 {
        vars.add("K", "v").unwrap();
    }
    assert_eq!(vars.iter().count(), 64);
    let snapshot = vars.clone();
    assert!(matches!(
        vars.add("NEXT", "v"),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(vars, snapshot);
}

#[ktest]
fn complete_events_reserve_base_fields_and_allocate_seqnum_after_validation() {
    // The ktest runner executes tests serially. Keep these snapshots adjacent
    // to builder calls, without registering devices or invoking broadcast hooks.
    let mut vars = UeventVars::new();
    vars.add("K", "v".repeat(2045)).unwrap();
    let before = uevent::current_seqnum();
    assert!(matches!(
        UeventBuilder::new(
            UeventAction::Change,
            "/devices/budget".into(),
            "test".into(),
            vars
        )
        .unwrap()
        .build(),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(uevent::current_seqnum(), before);

    let mut vars = UeventVars::new();
    for _ in 0..60 {
        vars.add("K", "v").unwrap();
    }
    UeventBuilder::new(
        UeventAction::Change,
        "/devices/budget".into(),
        "test".into(),
        vars.clone(),
    )
    .unwrap()
    .build()
    .unwrap();
    assert_eq!(uevent::current_seqnum(), before.wrapping_add(1));
    vars.add("K", "v").unwrap();
    let before = uevent::current_seqnum();
    assert!(matches!(
        UeventBuilder::new(
            UeventAction::Change,
            "/devices/budget".into(),
            "test".into(),
            vars
        )
        .unwrap()
        .build(),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(uevent::current_seqnum(), before);

    let seq_digits = before.wrapping_add(1).to_string().len();
    let base_bytes = "ACTION=change\0DEVPATH=/devices/budget\0SUBSYSTEM=test\0".len();
    let value_len = 2048 - base_bytes - "SEQNUM=\0".len() - seq_digits - "K=\0".len();
    let mut vars = UeventVars::new();
    vars.add("K", "v".repeat(value_len)).unwrap();
    UeventBuilder::new(
        UeventAction::Change,
        "/devices/budget".into(),
        "test".into(),
        vars,
    )
    .unwrap()
    .build()
    .unwrap();
    assert_eq!(uevent::current_seqnum(), before.wrapping_add(1));

    let before = uevent::current_seqnum();
    let seq_digits = before.wrapping_add(1).to_string().len();
    let value_len = 2048 - base_bytes - "SEQNUM=\0".len() - seq_digits - "K=\0".len();
    let mut oversized = UeventVars::new();
    oversized.add("K", "v".repeat(value_len + 1)).unwrap();
    assert!(matches!(
        UeventBuilder::new(
            UeventAction::Change,
            "/devices/budget".into(),
            "test".into(),
            oversized
        )
        .unwrap()
        .build(),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(uevent::current_seqnum(), before.wrapping_add(1));
}

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
    // The registered devnode policy is frozen even if the callback later changes.
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
    for key in ["ACTION=", "DEVPATH=", "SUBSYSTEM=", "SEQNUM="] {
        assert!(!environment.lines().any(|line| line.starts_with(key)));
    }
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
        (Some(0), Some("0")),
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
    let before = uevent::current_seqnum();
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
        Err(aster_systree::Error::MemoryBudgetExhausted)
    ));
    request.push_str(" K60=v K61=v K62=v K63=v");
    assert!(matches!(
        dev.store_attr("uevent", &request),
        Err(aster_systree::Error::InvalidOperation)
    ));
    assert_eq!(uevent::current_seqnum(), before);
    crate::remove_device(&dev).unwrap();
    assert!(matches!(
        dev.store_attr("uevent", "change"),
        Err(aster_systree::Error::IsDead)
    ));
}

#[ktest]
fn lifecycle_callbacks_observe_registration_and_driver_binding() {
    let (_, bus) = subsystems();
    let driver = bus.register_driver(Arc::new(EventDriver)).unwrap();
    let dev = BusDevice::builder(&bus, "lifecycle", Mutex::new(Vec::new())).build();
    let before = uevent::current_seqnum();
    assert!(matches!(
        crate::emit_uevent(&dev, UeventAction::Change),
        Err(Error::NotAdded)
    ));
    assert_eq!(uevent::current_seqnum(), before);
    crate::add_device(&dev).unwrap();
    assert_eq!(
        *dev.payload().lock(),
        vec![None, Some("event-driver".to_string())]
    );
    assert!(
        dev.show_attr("uevent")
            .unwrap()
            .contains("DRIVER=event-driver\n")
    );
    dev.payload().lock().clear();
    bus.unbind(&dev).unwrap();
    assert_eq!(*dev.payload().lock(), vec![None]);
    dev.payload().lock().clear();
    let driver_dir = utils::lookup("/bus/uevent-test-bus/drivers/event-driver")
        .unwrap()
        .cast_to_node()
        .unwrap();
    driver_dir.store_attr("bind", "lifecycle\n").unwrap();
    assert_eq!(
        *dev.payload().lock(),
        vec![Some("event-driver".to_string())]
    );
    crate::remove_device(&dev).unwrap();
    let before = uevent::current_seqnum();
    assert!(matches!(
        crate::emit_uevent(&dev, UeventAction::Change),
        Err(Error::NotAdded)
    ));
    assert_eq!(uevent::current_seqnum(), before);
    bus.unregister_driver(&driver).unwrap();
}

#[ktest]
fn callback_failure_is_best_effort_only_for_lifecycle_events() {
    let (class, _) = subsystems();
    let dev = ClassDevice::builder(&class, "callback-failure", NodePayload::new()).build();
    dev.fail_event.store(true, Ordering::Relaxed);
    let before = uevent::current_seqnum();
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
    assert_eq!(uevent::current_seqnum(), before);
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
    fn append_uevent_vars(&self, dev: &ClassDevice<Self>, vars: &mut UeventVars) -> Result<()> {
        if dev.fail_event.load(Ordering::Relaxed) {
            return Err(Error::Attribute);
        }
        vars.add("SOURCE", "class")
    }
}

static NODE_TYPE: DeviceType<ClassDevice<EventClass>> = {
    let mut dev_type = DeviceType::named("event-node");
    dev_type.append_uevent_vars_fn = Some(|_dev, vars| vars.add("SOURCE", "type"));
    dev_type
};

struct EventBus;

impl Bus for EventBus {
    const NAME: &'static str = "uevent-test-bus";
    type Device = Mutex<Vec<Option<String>>>;
    type MatchData = ();
    fn matches(&self, _dev: &Self::Device, _data: &Self::MatchData) -> bool {
        true
    }
    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>] {
        &[]
    }
    fn append_uevent_vars(&self, dev: &BusDevice<Self>, vars: &mut UeventVars) -> Result<()> {
        dev.payload()
            .lock()
            .push(dev.driver().map(|driver| driver.name().to_string()));
        vars.add("SOURCE", "bus")?;
        Ok(())
    }
}

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
