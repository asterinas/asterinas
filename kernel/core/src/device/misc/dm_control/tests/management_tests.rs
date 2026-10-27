// SPDX-License-Identifier: MPL-2.0

//! Tableless identity, selector, and list-record regressions.

use aster_device_mapper::{DmDevice, DmError, DmManager};
use ostd::prelude::ktest;

use super::{
    super::{
        DM_MANAGER,
        abi::*,
        commands::*,
        device_ops::{ControlWorkflow, CreateRequest, QueryWorkflow},
        manager,
    },
    request, test_buffer,
};
use crate::prelude::*;

fn list_devices_for_devices(buffer: &mut [u8], devices: Vec<Arc<DmDevice>>) -> Result<()> {
    list_devices_for_snapshots(
        buffer,
        devices.iter().map(|device| QueryWorkflow::device(device)),
        true,
    )
}

#[ktest]
fn device_selectors_ignore_high_encoding_bits() {
    let manager = DmManager::new().unwrap();
    let device = manager
        .create("dm-selector-high-bits".to_string(), None, Some(0xfffff))
        .unwrap();
    for high in [0, 1u64 << 32, u64::MAX << 32] {
        let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE);
        write_u64(&mut buffer, OFF_DEV, device.id().as_encoded_u64() | high).unwrap();
        assert!(Arc::ptr_eq(
            &lookup_device_in_manager(&buffer, &manager).unwrap(),
            &device
        ));
    }
    let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE);
    write_u64(&mut buffer, OFF_DEV, 1u64 << 32).unwrap();
    assert_eq!(
        lookup_device_in_manager(&buffer, &manager)
            .unwrap_err()
            .error(),
        Errno::ENXIO
    );
}

#[ktest]
fn rejects_ambiguous_selectors_without_selecting_either_device() {
    let manager = DmManager::new().unwrap();
    let first = manager
        .create(
            "dm-selector-first".to_string(),
            Some(b"dm-selector-uuid".to_vec()),
            None,
        )
        .unwrap();
    let second = manager
        .create("dm-selector-second".to_string(), None, None)
        .unwrap();
    for mask in 0..8 {
        let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE);
        if mask & 1 != 0 {
            write_c_string_fixed(&mut buffer, OFF_NAME, DM_NAME_LEN, second.mapper_name()).unwrap();
        }
        if mask & 2 != 0 {
            write_c_string_fixed(&mut buffer, OFF_UUID, DM_UUID_LEN, "dm-selector-uuid").unwrap();
        }
        if mask & 4 != 0 {
            write_u64(&mut buffer, OFF_DEV, second.id().as_encoded_u64()).unwrap();
        }
        let result = lookup_device_in_manager(&buffer, &manager);
        match mask {
            1 | 4 => assert!(Arc::ptr_eq(&result.unwrap(), &second)),
            2 => assert!(Arc::ptr_eq(&result.unwrap(), &first)),
            3 | 7 => assert_eq!(result.unwrap_err().error(), Errno::EINVAL),
            _ => assert_eq!(result.unwrap_err().error(), Errno::ENXIO),
        }
        assert!(manager.is_current(&first));
        assert!(manager.is_current(&second));
    }
}

#[ktest]
fn opaque_uuid_roundtrips_through_identity_and_list_records() {
    let manager = DmManager::new().unwrap();
    let uuid = b"opaque-\xff\x80";
    let device = manager
        .create("dm-opaque-uuid".to_string(), Some(uuid.to_vec()), None)
        .unwrap();
    let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE);
    write_c_string_fixed(&mut buffer, OFF_UUID, DM_UUID_LEN, uuid).unwrap();
    assert!(Arc::ptr_eq(
        &lookup_device_in_manager(&buffer, &manager).unwrap(),
        &device
    ));
    fill_device_header(&mut buffer, &device).unwrap();
    assert_eq!(
        optional_uuid(&buffer, OFF_UUID, DM_UUID_LEN)
            .unwrap()
            .unwrap(),
        uuid
    );

    let mut output = test_buffer(DM_IOCTL_HEADER_SIZE + 128);
    write_c_string_fixed(&mut output, OFF_UUID, DM_UUID_LEN, b"opaque-\xff").unwrap();
    list_devices_for_devices(&mut output, vec![device.clone()]).unwrap();
    let (extension, _) =
        name_list_record_layout(device.mapper_name().len() + 1, uuid.len() + 1).unwrap();
    assert_eq!(
        c_bytes_until(&output, DM_IOCTL_HEADER_SIZE + extension + 8, output.len()).unwrap(),
        uuid
    );
    assert!(Arc::ptr_eq(&manager.lookup_uuid(uuid).unwrap(), &device));
    manager.remove("dm-opaque-uuid").unwrap();
    assert!(manager.lookup_uuid(uuid).is_none());

    buffer[OFF_UUID..OFF_UUID + DM_UUID_LEN].fill(0xff);
    assert_eq!(
        optional_uuid(&buffer, OFF_UUID, DM_UUID_LEN)
            .unwrap_err()
            .error(),
        Errno::EINVAL
    );
}

#[ktest]
fn rejects_invalid_names_before_identity_changes() {
    for name in ["", ".", "..", "control", "a/b"] {
        assert_eq!(
            validate_mapper_name(name).unwrap_err().error(),
            Errno::EINVAL
        );
    }
    validate_mapper_name(&"a".repeat(DM_NAME_LEN - 1)).unwrap();
    assert_eq!(
        validate_mapper_name(&"a".repeat(DM_NAME_LEN))
            .unwrap_err()
            .error(),
        Errno::EINVAL
    );
    assert_eq!(map_create_error(DmError::NameExists).error(), Errno::EBUSY);
    assert_eq!(map_create_error(DmError::UuidExists).error(), Errno::EBUSY);
    assert_eq!(map_dm_error(DmError::NameExists).error(), Errno::EEXIST);
}

#[ktest]
fn lifecycle_guard_rejects_device_removed_before_operation() {
    let manager = DmManager::new().unwrap();
    let device = manager
        .create("dm-lifecycle-stale".to_string(), None, None)
        .unwrap();
    manager.remove(&device.mapper_name()).unwrap();
    device.drain_removed();
    let replacement = manager
        .create(
            "dm-lifecycle-stale".to_string(),
            None,
            Some(device.id().minor().get()),
        )
        .unwrap();
    assert_eq!(replacement.id(), device.id());

    assert_eq!(
        with_current_device_lifecycle_in(&manager, &device, |_| Ok(()))
            .unwrap_err()
            .error(),
        Errno::ENXIO
    );
}

#[ktest]
fn persistent_create_reports_allocation_before_identity_errors() {
    let manager = DmManager::new().unwrap();
    let workflow = ControlWorkflow::new(&manager);
    let device = workflow
        .create(CreateRequest {
            name: "dm-create-order".to_string(),
            uuid: Some(b"dm-create-order-uuid".to_vec()),
            requested_minor: Some(7),
        })
        .unwrap();
    for (name, uuid, minor, expected) in [
        ("dm-create-new", None, 7, Errno::ENXIO),
        ("dm-create-order", None, 7, Errno::ENXIO),
        (
            "dm-create-new",
            Some("dm-create-order-uuid"),
            7,
            Errno::ENXIO,
        ),
        ("dm-create-order", None, 8, Errno::EBUSY),
        (
            "dm-create-new",
            Some("dm-create-order-uuid"),
            8,
            Errno::EBUSY,
        ),
    ] {
        let result = workflow.create(CreateRequest {
            name: name.to_string(),
            uuid: uuid.map(|value| value.as_bytes().to_vec()),
            requested_minor: Some(minor),
        });
        assert_eq!(result.unwrap_err().error(), expected);
        assert_eq!(manager.devices().len(), 1);
        assert!(Arc::ptr_eq(
            &manager.lookup_name("dm-create-order").unwrap(),
            &device
        ));
        assert!(Arc::ptr_eq(
            &manager.lookup_uuid("dm-create-order-uuid").unwrap(),
            &device
        ));
        assert!(manager.lookup_name("dm-create-new").is_none());
    }
    let retry = workflow
        .create(CreateRequest {
            name: "dm-create-new".to_string(),
            uuid: None,
            requested_minor: Some(8),
        })
        .unwrap();
    assert_eq!(retry.id().minor().get(), 8);
    assert_eq!(
        map_create_error(DmError::MinorExhausted).error(),
        Errno::ENXIO
    );
}

#[ktest]
fn create_device_does_not_persist_readonly_flag_without_a_table() {
    DM_MANAGER.call_once(|| DmManager::new().unwrap());
    let name = "dm-readonly-create-test";
    let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE);
    write_u32(&mut buffer, OFF_FLAGS, DM_READONLY_FLAG).unwrap();
    write_c_string_fixed(&mut buffer, OFF_NAME, DM_NAME_LEN, name).unwrap();

    create_device(&mut buffer).unwrap();
    let device = manager().lookup_name(name).unwrap();

    assert_eq!(read_u32(&buffer, OFF_FLAGS).unwrap(), 0);

    assert!(aster_block::lookup(device.id()).is_none());

    manager().remove(name).unwrap();
}

#[ktest]
fn lists_devices_with_linux_next_offsets_and_uuid_flags() {
    let manager = DmManager::new().unwrap();
    let first = manager
        .create(
            "dm-list-first".to_string(),
            Some(b"dm-list-first-uuid".to_vec()),
            None,
        )
        .unwrap();
    let second = manager
        .create("dm-list-second".to_string(), None, None)
        .unwrap();

    let first_name = first.mapper_name();
    let first_uuid = first.uuid().unwrap();
    let (first_extension, first_len) =
        name_list_record_layout(first_name.len() + 1, first_uuid.len() + 1).unwrap();
    let second_name = second.mapper_name();
    let (second_extension, second_len) = name_list_record_layout(second_name.len() + 1, 0).unwrap();
    let mut buffer = test_buffer(DM_IOCTL_HEADER_SIZE + first_len + second_len);

    list_devices_for_devices(&mut buffer, vec![first.clone(), second.clone()]).unwrap();

    let first_record = DM_IOCTL_HEADER_SIZE;
    assert_eq!(
        read_u64(&buffer, first_record).unwrap(),
        first.id().as_encoded_u64()
    );
    assert_eq!(
        read_u32(&buffer, first_record + 8).unwrap(),
        first_len as u32
    );
    assert_eq!(
        c_bytes_until(&buffer, first_record + 12, first_record + first_extension).unwrap(),
        first_name.as_bytes()
    );
    assert_eq!(
        read_u32(&buffer, first_record + first_extension).unwrap(),
        0
    );
    assert_eq!(
        read_u32(&buffer, first_record + first_extension + 4).unwrap(),
        DM_NAME_LIST_FLAG_HAS_UUID
    );
    assert_eq!(
        c_bytes_until(
            &buffer,
            first_record + first_extension + 8,
            first_record + first_len
        )
        .unwrap(),
        first_uuid
    );

    let second_record = DM_IOCTL_HEADER_SIZE + first_len;
    assert_eq!(
        read_u64(&buffer, second_record).unwrap(),
        second.id().as_encoded_u64()
    );
    assert_eq!(read_u32(&buffer, second_record + 8).unwrap(), 0);
    assert_eq!(
        c_bytes_until(
            &buffer,
            second_record + 12,
            second_record + second_extension
        )
        .unwrap(),
        second_name.as_bytes()
    );
    assert_eq!(
        read_u32(&buffer, second_record + second_extension).unwrap(),
        0
    );
    assert_eq!(
        read_u32(&buffer, second_record + second_extension + 4).unwrap(),
        DM_NAME_LIST_FLAG_DOESNT_HAVE_UUID
    );
}

#[ktest]
fn name_lists_filter_and_keep_events_without_uuid_output() {
    let manager = DmManager::new().unwrap();
    let first = manager
        .create("v5-list-a".to_string(), Some(b"v5-uuid-a".to_vec()), None)
        .unwrap();
    let second = manager.create("v5-list-b".to_string(), None, None).unwrap();
    for include_uuid in [false, true] {
        let mut output = request(512);
        write_c_string_fixed(&mut output, OFF_NAME, DM_NAME_LEN, "v5-list-").unwrap();
        write_c_string_fixed(&mut output, OFF_UUID, DM_UUID_LEN, "v5-uuid-").unwrap();
        output[305..].fill(0xa5);
        list_devices_for_snapshots(
            &mut output,
            [first.clone(), second.clone()]
                .iter()
                .map(|device| QueryWorkflow::device(device)),
            include_uuid,
        )
        .unwrap();
        // Independent byte offsets: name is 10 bytes including NUL, extension
        // is align8(12 + 10) == 24, then event and flags occupy 8 bytes.
        assert_eq!(read_u64(&output, 312).unwrap(), first.id().as_encoded_u64());
        assert_eq!(read_u32(&output, 320).unwrap(), 0);
        assert_eq!(read_u32(&output, 336).unwrap(), 0);
        assert_eq!(read_u32(&output, 340).unwrap(), u32::from(include_uuid));
        assert_eq!(
            read_u32(&output, OFF_DATA_SIZE).unwrap(),
            if include_uuid { 360 } else { 344 }
        );
        assert!(output[305..312].iter().all(|byte| *byte == 0));
    }
    let mut empty = request(320);
    write_c_string_fixed(&mut empty, OFF_NAME, DM_NAME_LEN, "no-match").unwrap();
    empty[312..].fill(0xa5);
    list_devices_for_snapshots(
        &mut empty,
        manager
            .devices()
            .iter()
            .map(|device| QueryWorkflow::device(device)),
        false,
    )
    .unwrap();
    assert_eq!(read_u32(&empty, OFF_DATA_SIZE).unwrap(), 312);
    assert_eq!(read_u64(&empty, 312).unwrap(), 0);
    // The sentinel is prepared in kernel memory but lies outside Linux's empty
    // response length. Raw C tests check the corresponding copyout boundary.
    let mut short = request(343);
    short[312..].fill(0xa5);
    list_devices_for_snapshots(
        &mut short,
        [first].iter().map(|device| QueryWorkflow::device(device)),
        false,
    )
    .unwrap();
    assert_ne!(
        read_u32(&short, OFF_FLAGS).unwrap() & DM_BUFFER_FULL_FLAG,
        0
    );
    assert!(short[312..].iter().all(|byte| *byte == 0xa5));
}
