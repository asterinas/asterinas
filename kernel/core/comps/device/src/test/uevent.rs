// SPDX-License-Identifier: MPL-2.0

use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use aster_systree::SysNode;
use device_id::{DeviceId, MajorId, MinorId};
use ostd::prelude::ktest;

use crate::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{Attr, DevNode, DevNum, DeviceType, Error, Result},
    uevent::{self, UeventAction, UeventBuilder, UeventVars},
};

#[ktest]
fn synthetic_environment_preserves_uuid_and_duplicate_argument_order() {
    // 1. Check that parsing preserves UUID case and the order of repeated arguments.
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

    // 2. Parse action-only requests. The omitted UUID becomes "0" without arguments.
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
    // 1. Supply requests that violate action, UUID, or argument syntax.
    for request in [
        "",
        "unknown",
        "change ",
        "change\t",
        "change  12345678-1234-1234-1234-123456789abc",
        "change 12345678-1234-1234-1234-123456789ab",
        "change 12345678-1234-1234-1234-123456789abg",
        "change FOO=1",
        "change 12345678-1234-1234-1234-123456789abc FOO",
        "change 12345678-1234-1234-1234-123456789abc =1",
        "change 12345678-1234-1234-1234-123456789abc FOO=",
        "change 12345678-1234-1234-1234-123456789abc F_OO=1",
    ] {
        // 2. Each malformed request should return InvalidValue.
        assert!(
            matches!(uevent::parse_synthetic(request), Err(Error::InvalidValue)),
            "{request:?}"
        );
    }
}

#[ktest]
fn synthetic_environment_overflow_maps_to_invalid_value() {
    // 1. Fill 64 entries including SYNTH_UUID, then reject one extra argument.
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

    // 2. Fill 2048 serialized bytes including NULs, then reject one extra byte.
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
fn variable_validation_and_limits_preserve_state() {
    // 1. Reject invalid keys and values before mutating the existing environment.
    let mut vars = UeventVars::new();
    vars.add("KEEP", "value").unwrap();
    let expected = vars.clone();
    for (key, value) in [("", "x"), ("K=V", "x"), ("K\0", "x"), ("K", "x\0y")] {
        assert!(matches!(vars.add(key, value), Err(Error::InvalidValue)));
        assert_eq!(vars, expected);
    }

    // 2. Reject an oversized value before insertion, preserving KEEP and byte usage.
    assert!(matches!(
        vars.add("BIG", "v".repeat(2048)),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(vars, expected);

    // 3. Fill the byte budget exactly and check that a rejected addition preserves state.
    let mut vars = UeventVars::new();
    vars.add("K", "v".repeat(2045)).unwrap();
    assert_eq!(vars.total_bytes(), 2048);
    let snapshot = vars.clone();
    assert!(matches!(
        vars.add("NEXT", "v"),
        Err(Error::MemoryBudgetExhausted)
    ));
    assert_eq!(vars, snapshot);

    // 4. Fill 64 entries and reject the 65th without changing entries or byte usage.
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
fn events_allocate_sequence_number_only_after_preceding_fields_fit() {
    let class = event_class();
    let dev = ClassDevice::builder(&class, "budget", NodePayload::new()).build();
    let dev_vars = UeventVars::from_dev(dev.as_ref()).unwrap();

    // 1. Exhaust bytes before SEQNUM is appended. This failure consumes no number.
    let before = uevent::current_seqnum();
    let mut builder = UeventBuilder::from_dev(dev.as_ref(), UeventAction::Change)
        .unwrap()
        .unwrap();
    let mut oversized = UeventVars::new();
    oversized
        .add(
            "K",
            "v".repeat(2048 - dev_vars.total_bytes() - b"K=\0".len()),
        )
        .unwrap();
    builder.append_vars(oversized).unwrap();
    assert!(matches!(builder.build(), Err(Error::MemoryBudgetExhausted)));
    assert_eq!(uevent::current_seqnum(), before);

    // 2. Build a valid event, consuming exactly one sequence number.
    let mut builder = UeventBuilder::from_dev(dev.as_ref(), UeventAction::Change)
        .unwrap()
        .unwrap();
    let mut vars = UeventVars::new();
    vars.add("K", "v").unwrap();
    builder.append_vars(vars).unwrap();
    builder.build().unwrap();
    assert_eq!(uevent::current_seqnum(), before.wrapping_add(1));

    // 3. Leave no entry for SEQNUM. Allocation precedes the failed insertion,
    // so this failure still consumes one number.
    let before = uevent::current_seqnum();
    let mut builder = UeventBuilder::from_dev(dev.as_ref(), UeventAction::Change)
        .unwrap()
        .unwrap();
    let mut max_vars = UeventVars::new();
    for _ in 0..61 - dev_vars.iter().count() {
        max_vars.add("K", "v").unwrap();
    }
    builder.append_vars(max_vars).unwrap();
    assert!(matches!(builder.build(), Err(Error::MemoryBudgetExhausted)));
    assert_eq!(uevent::current_seqnum(), before.wrapping_add(1));
}

#[ktest]
fn sysfs_reports_frozen_devnode_policy_and_ordered_callbacks() {
    // 1. Change the devnode callback after registration has frozen the node policy.
    let class = event_class();
    let dev = ClassDevice::builder(&class, "env!node", NodePayload::new())
        .devnum(DevNum::char(DeviceId::new(
            MajorId::new(240),
            MinorId::new(1),
        )))
        .dev_type(&NODE_TYPE)
        .build();
    crate::add_device(&dev).unwrap();
    dev.change_node_name.store(true, Ordering::Relaxed);

    // 2. Check the saved devnode settings and class-before-type callback order.
    // Reading sysfs leaves the sequence number unchanged and omits event-only fields.
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

    // 3. Remove the device and check that its dead sysfs node rejects further reads.
    crate::remove_device(&dev).unwrap();
    assert!(matches!(
        dev.show_attr("uevent"),
        Err(aster_systree::Error::IsDead)
    ));
}

#[ktest]
fn explicit_events_require_a_registered_device() {
    let class = event_class();
    let dev = ClassDevice::builder(&class, "explicit-event", NodePayload::new()).build();
    let before = uevent::current_seqnum();
    assert!(matches!(
        uevent::emit(dev.as_ref(), UeventAction::Change),
        Err(Error::NotAdded)
    ));
    assert_eq!(uevent::current_seqnum(), before);

    crate::add_device(&dev).unwrap();
    crate::remove_device(&dev).unwrap();
    let before = uevent::current_seqnum();
    assert!(matches!(
        uevent::emit(dev.as_ref(), UeventAction::Change),
        Err(Error::NotAdded)
    ));
    assert_eq!(uevent::current_seqnum(), before);
}

#[ktest]
fn callback_failure_is_best_effort_only_for_lifecycle_events() {
    // 1. Fail the environment callback. Best-effort add must still register the device.
    let class = event_class();
    let dev = ClassDevice::builder(&class, "callback-failure", NodePayload::new()).build();
    dev.fail_event.store(true, Ordering::Relaxed);
    let before = uevent::current_seqnum();
    crate::add_device(&dev).unwrap();
    assert!(class.find_device("callback-failure").is_some());

    // 2. Explicit emission and sysfs access must expose the same callback failure.
    assert!(matches!(
        uevent::emit(dev.as_ref(), UeventAction::Change),
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

    // 3. Automatic remove is also best-effort. All callbacks failed before event
    // construction, so neither lifecycle nor explicit attempts consumed a number.
    crate::remove_device(&dev).unwrap();
    assert!(class.find_device("callback-failure").is_none());
    assert_eq!(uevent::current_seqnum(), before);
}

fn event_class() -> Arc<ClassHandle<EventClass>> {
    static CLASS: spin::Once<Arc<ClassHandle<EventClass>>> = spin::Once::new();
    crate::init_for_ktest();
    CLASS
        .call_once(|| class::register(EventClass).unwrap())
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
