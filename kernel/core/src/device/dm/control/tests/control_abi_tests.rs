// SPDX-License-Identifier: MPL-2.0

//! Control-file readiness, version negotiation, and command decoding.

use ostd::prelude::ktest;

use super::super::abi::*;
use crate::{device::dm::DmControlFile, events::IoEvents, prelude::*, process::signal::Pollable};

#[ktest]
fn control_without_events_is_not_ready() {
    let file = DmControlFile;
    for mask in [
        IoEvents::empty(),
        IoEvents::IN,
        IoEvents::OUT,
        IoEvents::IN | IoEvents::OUT,
    ] {
        assert_eq!(file.poll(mask, None), IoEvents::empty());
    }
}

#[ktest]
fn accepts_backward_compatible_interface_versions() {
    for version in [[4, 0, 0], [4, 48, 0], [4, 48, u32::MAX]] {
        let mut buffer = [0u8; 12];
        for (index, value) in version.into_iter().enumerate() {
            write_u32(&mut buffer, index * 4, value).unwrap();
        }
        validate_client_version(&buffer).unwrap();
    }
}

#[ktest]
fn rejects_incompatible_interface_versions() {
    for version in [
        [3, 0, 0],
        [5, 0, 0],
        [4, 49, 0],
        [4, 50, 0],
        [4, u32::MAX, 0],
    ] {
        let mut buffer = [0u8; 12];
        for (index, value) in version.into_iter().enumerate() {
            write_u32(&mut buffer, index * 4, value).unwrap();
        }
        assert_eq!(
            validate_client_version(&buffer).unwrap_err().error(),
            Errno::EINVAL
        );
    }
}

#[ktest]
fn accepts_version_regardless_of_direction_and_size() {
    for raw in [0xc138_fd00, 0x0000_fd00, 0x8004_fd00, 0xffff_fd00] {
        assert_eq!(decode_command(raw).unwrap(), DM_VERSION_CMD);
    }
}

#[ktest]
fn rejects_wrong_type_and_unsupported_commands() {
    for raw in [0, 0xc138_fe00, 0xc138_fd01, 0xc138_fd11, 0xc138_fdff] {
        assert_eq!(decode_command(raw).unwrap_err().error(), Errno::ENOTTY);
    }
}

#[ktest]
fn writes_only_the_server_version_fields() {
    let mut buffer = [0xa5; 16];
    write_version(&mut buffer).unwrap();
    for (index, value) in DM_VERSION.into_iter().enumerate() {
        assert_eq!(read_u32(&buffer, index * 4).unwrap(), value);
    }
    assert_eq!(&buffer[12..], &[0xa5; 4]);
}

#[ktest]
fn bounds_version_field_accesses() {
    assert_eq!(
        validate_client_version(&[0; 3]).unwrap_err().error(),
        Errno::EINVAL
    );
    assert_eq!(
        write_version(&mut [0; 11]).unwrap_err().error(),
        Errno::EINVAL
    );
    assert_eq!(
        read_u32(&[0; 12], usize::MAX).unwrap_err().error(),
        Errno::EINVAL
    );
    assert_eq!(
        write_u32(&mut [0; 12], usize::MAX, 0u32)
            .unwrap_err()
            .error(),
        Errno::EINVAL
    );
}
