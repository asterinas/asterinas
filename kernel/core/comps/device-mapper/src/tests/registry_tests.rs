// SPDX-License-Identifier: MPL-2.0

//! Identity indexes, minor allocation, and creation failure rollback.

use alloc::{string::ToString, sync::Arc};

use ostd::prelude::ktest;

use crate::{DmError, DmManager};

#[ktest]
fn creation_allocates_minor_before_identity_checks_and_releases_on_failure() {
    let manager = DmManager::new().unwrap();
    let device = manager
        .create(
            "existing".to_string(),
            Some(b"existing-uuid".to_vec()),
            Some(7),
        )
        .unwrap();
    for (name, uuid, identity_error) in [
        ("existing", None, DmError::NameExists),
        ("other", Some("existing-uuid"), DmError::UuidExists),
    ] {
        assert_eq!(
            manager
                .create(
                    name.to_string(),
                    uuid.map(|value| value.as_bytes().to_vec()),
                    Some(7)
                )
                .unwrap_err(),
            DmError::MinorBusy
        );
        assert_eq!(
            manager
                .create(
                    name.to_string(),
                    uuid.map(|value| value.as_bytes().to_vec()),
                    Some(8)
                )
                .unwrap_err(),
            identity_error
        );
        assert_eq!(manager.devices().len(), 1);
        assert!(Arc::ptr_eq(
            &manager.lookup_name("existing").unwrap(),
            &device
        ));
        assert!(Arc::ptr_eq(
            &manager.lookup_uuid("existing-uuid").unwrap(),
            &device
        ));
        assert!(manager.lookup_name("other").is_none());
        let retry = manager.create("retry".to_string(), None, Some(8)).unwrap();
        assert_eq!(retry.id().minor().get(), 8);
        drop(manager.remove("retry").unwrap());
        drop(retry);
    }
}

#[ktest]
fn manages_indexes_and_requested_minors() {
    let manager = DmManager::new().unwrap();
    let device = manager
        .create("dm-test".to_string(), Some(b"uuid-test".to_vec()), Some(7))
        .unwrap();

    assert_eq!(device.id().minor().get(), 7);
    assert_eq!(manager.lookup_name("dm-test").unwrap().id(), device.id());
    assert_eq!(manager.lookup_uuid("uuid-test").unwrap().id(), device.id());
    assert_eq!(manager.lookup_id(device.id()).unwrap().id(), device.id());
    assert_eq!(manager.devices().len(), 1);
    assert_eq!(
        manager
            .create("dm-test".to_string(), None, None)
            .unwrap_err(),
        DmError::NameExists
    );
    assert_eq!(
        manager
            .create("dm-other".to_string(), Some(b"uuid-test".to_vec()), None,)
            .unwrap_err(),
        DmError::UuidExists
    );
    assert_eq!(
        manager
            .create("dm-other".to_string(), None, Some(7))
            .unwrap_err(),
        DmError::MinorBusy
    );
}
