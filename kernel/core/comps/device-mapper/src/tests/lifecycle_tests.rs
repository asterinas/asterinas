// SPDX-License-Identifier: MPL-2.0

//! Control-reference drain and device-number ownership across removal.

use alloc::{string::ToString, sync::Arc};
use core::sync::atomic::{AtomicBool, Ordering};

use device_id::MajorId;
use ostd::{
    prelude::ktest,
    task::{Task, TaskOptions},
};

use crate::{DmError, DmManager};

#[ktest]
fn removal_drains_admitted_controls_but_not_structural_snapshots() {
    for create_holder in [false, true] {
        let manager = DmManager::new().unwrap();
        let initial = manager
            .create_control("drain".to_string(), None, Some(7))
            .unwrap();
        let device = Arc::clone(&initial);
        let holder = if create_holder {
            initial
        } else {
            drop(initial);
            manager.acquire_control(&device).unwrap()
        };
        {
            let _lifecycle = device.lock_lifecycle();
            manager.remove("drain").unwrap();
        }
        assert!(manager.acquire_control(&device).is_none());
        let started = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        {
            let device = device.clone();
            let started = started.clone();
            let finished = finished.clone();
            TaskOptions::new(move || {
                started.store(true, Ordering::Release);
                device.drain_removed();
                finished.store(true, Ordering::Release);
            })
            .spawn()
            .unwrap();
        }
        while !started.load(Ordering::Acquire) {
            Task::yield_now();
        }
        assert!(!finished.load(Ordering::Acquire));
        assert_eq!(
            manager
                .create("retry".to_string(), None, Some(7))
                .unwrap_err(),
            DmError::MinorBusy
        );
        // An admitted operation can still acquire the lifecycle lock and inspect
        // identity while REMOVE is waiting outside that lock.
        {
            let _lifecycle = holder.lock_lifecycle();
            assert_eq!(holder.id(), device.id());
        }
        drop(holder);
        while !finished.load(Ordering::Acquire) {
            Task::yield_now();
        }
        let replacement = manager.create("retry".to_string(), None, Some(7)).unwrap();
        assert_eq!(device.id(), replacement.id());
        drop(device);
        assert_eq!(
            manager
                .create("collision".to_string(), None, Some(7))
                .unwrap_err(),
            DmError::MinorBusy
        );
    }
}

#[ktest]
fn completed_removal_releases_minor_without_dropping_old_snapshots() {
    let manager = DmManager::new().unwrap();
    let old = manager
        .create("dm-reuse".to_string(), None, Some(7))
        .unwrap();
    let id = old.id();
    {
        let _lifecycle = old.lock_lifecycle();

        manager.remove("dm-reuse").unwrap();
    }
    old.drain_removed();
    let replacement = manager
        .create("dm-reuse".to_string(), None, Some(7))
        .unwrap();
    assert_eq!(replacement.id(), id);
    assert_eq!(old.id(), id);
    assert!(!manager.is_current(&old));
    assert!(manager.acquire_control(&old).is_none());
    assert!(Arc::ptr_eq(&manager.lookup_id(id).unwrap(), &replacement));
    // Repeated cleanup and the old object's drop must not release the new
    // allocation. Structural snapshots keep identity, not minor ownership.
    old.drain_removed();
    drop(old);
    assert_eq!(
        manager
            .create("dm-collision".to_string(), None, Some(7))
            .unwrap_err(),
        DmError::MinorBusy
    );
    {
        let _lifecycle = replacement.lock_lifecycle();

        manager.remove("dm-reuse").unwrap();
    }
    replacement.drain_removed();
    assert!(manager.create("dm-next".to_string(), None, Some(7)).is_ok());
}

#[ktest]
fn releases_persistent_minor_after_last_device_reference_drops() {
    let manager = DmManager::new().unwrap();
    let device = manager
        .create("dm-persistent-minor".to_string(), None, Some(7))
        .unwrap();
    let removed = manager.remove("dm-persistent-minor").unwrap();

    assert_eq!(
        manager
            .create("dm-persistent-minor-retry".to_string(), None, Some(7))
            .unwrap_err(),
        DmError::MinorBusy
    );

    drop(device);
    drop(removed);
    assert!(
        manager
            .create("dm-persistent-minor-retry".to_string(), None, Some(7))
            .is_ok()
    );
}

#[ktest]
fn keeps_device_ids_owned_until_last_device_reference_drops() {
    let manager = DmManager::new().unwrap();
    let major = manager.major();
    let device = manager
        .create("dm-test".to_string(), None, Some(7))
        .unwrap();
    let removed = manager.remove("dm-test").unwrap();

    assert!(manager.lookup_name("dm-test").is_none());
    assert_eq!(
        manager
            .create("dm-replacement".to_string(), None, Some(7))
            .unwrap_err(),
        DmError::MinorBusy
    );

    drop(manager);
    assert!(aster_block::acquire_major(MajorId::new(major)).is_err());
    drop(device);
    drop(removed);

    let _major_owner = aster_block::acquire_major(MajorId::new(major)).unwrap();
}
