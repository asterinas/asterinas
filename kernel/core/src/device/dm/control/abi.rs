// SPDX-License-Identifier: MPL-2.0

//! Device Mapper ioctl ABI for CREATE, LOAD, suspend/resume, REMOVE and static queries.

use crate::prelude::*;

pub(super) const DM_LIST_DEVICES_CMD: u8 = 2;
pub(super) const DM_DEV_CREATE_CMD: u8 = 3;
pub(super) const DM_DEV_REMOVE_CMD: u8 = 4;
pub(super) const DM_DEV_SUSPEND_CMD: u8 = 6;
pub(super) const DM_DEV_STATUS_CMD: u8 = 7;
pub(super) const DM_TABLE_LOAD_CMD: u8 = 9;
pub(super) const DM_TABLE_CLEAR_CMD: u8 = 10;
pub(super) const DM_TABLE_DEPS_CMD: u8 = 11;
pub(super) const DM_TABLE_STATUS_CMD: u8 = 12;
pub(super) const DM_LIST_VERSIONS_CMD: u8 = 13;
pub(super) const DM_GET_TARGET_VERSION_CMD: u8 = 17;
pub(super) const FIXED_PREFIX_SIZE: usize = 305;
pub(super) const MAX_IOCTL_SIZE: u32 = 1024 * 1024 * 1024;
pub(super) const OFF_DATA_SIZE: usize = 12;
pub(super) const OFF_FLAGS: usize = 28;
pub(super) const OFF_NAME: usize = 48;
pub(super) const NAME_LEN: usize = 128;
pub(super) const OFF_UUID: usize = 176;
pub(super) const UUID_LEN: usize = 129;
pub(super) const BUFFER_FULL_FLAG: u32 = 1 << 8;
pub(super) const UEVENT_GENERATED_FLAG: u32 = 1 << 13;
pub(super) const SECURE_DATA_FLAG: u32 = 1 << 15;
pub(super) const DATA_OUT_FLAG: u32 = 1 << 16;

pub(super) fn validate_ioctl_size(size: u32) -> Result<()> {
    if !(FIXED_PREFIX_SIZE as u32..=MAX_IOCTL_SIZE).contains(&size) {
        return_errno_with_message!(Errno::EINVAL, "invalid device-mapper data_size");
    }
    Ok(())
}

pub(super) fn validate_ioctl_params(
    buffer: &mut [u8; FIXED_PREFIX_SIZE],
    command: u8,
) -> Result<()> {
    let flags = read_u32(buffer, OFF_FLAGS)
        & !(BUFFER_FULL_FLAG | UEVENT_GENERATED_FLAG | SECURE_DATA_FLAG | DATA_OUT_FLAG);
    write_u32(buffer, OFF_FLAGS, flags);
    if command == DM_DEV_CREATE_CMD {
        if buffer[OFF_NAME] == 0 {
            return_errno_with_message!(Errno::EINVAL, "missing mapper name");
        }
    } else if command != DM_LIST_DEVICES_CMD && buffer[OFF_NAME] != 0 && buffer[OFF_UUID] != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "name and UUID selectors are mutually exclusive"
        );
    }
    buffer[OFF_NAME + NAME_LEN - 1] = 0;
    buffer[OFF_UUID + UUID_LEN - 1] = 0;
    Ok(())
}

pub(super) fn read_u32(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
}

pub(super) fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
