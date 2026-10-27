// SPDX-License-Identifier: MPL-2.0

//! Linux DM wire layout, bounded field access, validation, and errno mapping.
//!
//! Helpers operate on copied byte slices without accessing userspace or indexes.

use aster_device_mapper::DmError;
use device_id::{DeviceId, MajorId, MinorId};

use crate::prelude::*;

pub(super) const DM_IOCTL_MAGIC: u32 = 0xfd;
// Input stops at offsetof(dm_ioctl, data); output records follow sizeof(dm_ioctl).
pub(super) const DM_IOCTL_FIXED_PREFIX_SIZE: usize = 305;
pub(super) const DM_IOCTL_HEADER_SIZE: usize = 312;
// Linux DM_MAX_TARGETS (1048576) * DM_MAX_TARGET_PARAMS (1024).
pub(super) const DM_IOCTL_MAX_SIZE: usize = 1024 * 1024 * 1024;
pub(super) const DM_VERSION: [u32; 3] = [4, 48, 0];
pub(super) const DM_VERSION_CMD: u8 = 0;
pub(super) const DM_LIST_DEVICES_CMD: u8 = 2;
pub(super) const DM_DEV_CREATE_CMD: u8 = 3;
pub(super) const DM_DEV_REMOVE_CMD: u8 = 4;
pub(super) const DM_DEV_STATUS_CMD: u8 = 7;
pub(super) const DM_READONLY_FLAG: u32 = 1 << 0;
pub(super) const DM_SUSPEND_FLAG: u32 = 1 << 1;
pub(super) const DM_PERSISTENT_DEV_FLAG: u32 = 1 << 3;
pub(super) const DM_ACTIVE_PRESENT_FLAG: u32 = 1 << 5;
pub(super) const DM_INACTIVE_PRESENT_FLAG: u32 = 1 << 6;
pub(super) const DM_BUFFER_FULL_FLAG: u32 = 1 << 8;
pub(super) const DM_UEVENT_GENERATED_FLAG: u32 = 1 << 13;
pub(super) const DM_UUID_FLAG: u32 = 1 << 14;
pub(super) const DM_SECURE_DATA_FLAG: u32 = 1 << 15;
pub(super) const DM_DATA_OUT_FLAG: u32 = 1 << 16;
pub(super) const DM_DEFERRED_REMOVE: u32 = 1 << 17;
pub(super) const DM_INTERNAL_SUSPEND_FLAG: u32 = 1 << 18;
pub(super) const DM_NAME_LIST_FLAG_HAS_UUID: u32 = 1;
pub(super) const DM_NAME_LIST_FLAG_DOESNT_HAVE_UUID: u32 = 2;
const OFF_VERSION: usize = 0;
pub(super) const OFF_DATA_SIZE: usize = 12;
pub(super) const OFF_DATA_START: usize = 16;
pub(super) const OFF_TARGET_COUNT: usize = 20;
pub(super) const OFF_OPEN_COUNT: usize = 24;
pub(super) const OFF_FLAGS: usize = 28;
pub(super) const OFF_EVENT_NR: usize = 32;
pub(super) const OFF_DEV: usize = 40;
pub(super) const OFF_NAME: usize = 48;
pub(super) const DM_NAME_LEN: usize = 128;
pub(super) const OFF_UUID: usize = 176;
pub(super) const DM_UUID_LEN: usize = 129;

/// Bounds allocations independently of the command's data offset.
///
/// Matches Linux's 1 GiB ABI ceiling, not a promise of allocation success.
/// NO_PARAMS commands validate this bound but only allocate the fixed prefix.
pub(super) fn validate_ioctl_size(data_size: usize) -> Result<()> {
    if !(DM_IOCTL_FIXED_PREFIX_SIZE..=DM_IOCTL_MAX_SIZE).contains(&data_size) {
        return_errno_with_message!(Errno::EINVAL, "dm_ioctl.data_size exceeds supported bounds");
    }
    Ok(())
}

/// Accepts the same major and a minor no newer than this implementation's ABI.
///
/// Linux 6.6 `check_version` (5078a3a23a1e) treats minor revisions as backward compatible
/// and ignores patch differences. The caller must report the kernel version to
/// userspace even when this check rejects the client's requested version.
pub(super) fn validate_client_version(buffer: &[u8]) -> Result<()> {
    if read_u32(buffer, OFF_VERSION)? != DM_VERSION[0]
        || read_u32(buffer, OFF_VERSION + 4)? > DM_VERSION[1]
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "incompatible device-mapper interface version"
        );
    }
    Ok(())
}

/// Reads a bounded byte field for list prefix filtering without allocating.
pub(super) fn fixed_bytes(buffer: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    require_range(buffer, offset, len)?;
    let bytes = &buffer[offset..offset + len];
    Ok(&bytes[..bytes.iter().position(|byte| *byte == 0).unwrap_or(len)])
}

/// Computes aligned offsets for one `DM_LIST_DEVICES` name-list record.
pub(super) fn name_list_record_layout(name_len: usize, uuid_len: usize) -> Result<(usize, usize)> {
    let extension_offset = align_up(12usize.checked_add(name_len).ok_or_else(invalid_buffer)?, 8)?;
    let record_end = extension_offset
        .checked_add(8)
        .and_then(|value| value.checked_add(uuid_len))
        .ok_or_else(invalid_buffer)?;
    Ok((extension_offset, align_up(record_end, 8)?))
}

/// Selects Linux's aligned result area independently of the input data_start.
pub(super) fn result_start(buffer: &mut [u8]) -> Result<usize> {
    // The seven padding bytes between the fixed prefix and result records were
    // copied from userspace. Never return that input padding in secure output.
    let end = buffer.len().min(DM_IOCTL_HEADER_SIZE);
    buffer[DM_IOCTL_FIXED_PREFIX_SIZE..end].fill(0);
    write_u32(buffer, OFF_DATA_START, DM_IOCTL_HEADER_SIZE)?;
    Ok(DM_IOCTL_HEADER_SIZE)
}

pub(super) fn available_from(buffer: &[u8], start: usize) -> usize {
    buffer.len().saturating_sub(start)
}

pub(super) fn align_up(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(invalid_buffer)
}

/// Sets `DM_BUFFER_FULL_FLAG` when the caller-provided output area is too small.
pub(super) fn set_buffer_full(buffer: &mut [u8]) -> Result<()> {
    let flags = read_u32(buffer, OFF_FLAGS)? | DM_BUFFER_FULL_FLAG;
    write_u32(buffer, OFF_FLAGS, flags)
}

pub(super) fn write_version(buffer: &mut [u8]) -> Result<()> {
    for (index, value) in DM_VERSION.into_iter().enumerate() {
        write_u32(buffer, OFF_VERSION + index * 4, value)?;
    }
    Ok(())
}

/// Reads a required fixed-size NUL-terminated string from an ABI field.
pub(super) fn required_c_string(buffer: &[u8], offset: usize, len: usize) -> Result<String> {
    optional_c_string(buffer, offset, len)?
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "required mapper field is empty"))
}

/// Reads an optional fixed-size NUL-terminated string from an ABI field.
pub(super) fn optional_c_string(
    buffer: &[u8],
    offset: usize,
    len: usize,
) -> Result<Option<String>> {
    require_range(buffer, offset, len)?;
    let bytes = &buffer[offset..offset + len];
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(Error::with_message(
            Errno::EINVAL,
            "mapper string field is not NUL-terminated",
        ));
    };
    if end == 0 {
        return Ok(None);
    }
    let value = core::str::from_utf8(&bytes[..end])
        .map_err(|_| Error::with_message(Errno::EINVAL, "mapper name is not valid UTF-8"))?;
    Ok(Some(value.to_string()))
}

/// Reads an optional opaque UUID, requiring NUL but not UTF-8.
pub(super) fn optional_uuid(buffer: &[u8], offset: usize, len: usize) -> Result<Option<Vec<u8>>> {
    require_range(buffer, offset, len)?;
    let bytes = c_bytes_until(buffer, offset, offset + len)?;
    Ok((!bytes.is_empty()).then(|| bytes.to_vec()))
}

/// Borrows one bounded NUL-terminated byte string without interpreting its encoding.
pub(super) fn c_bytes_until(buffer: &[u8], start: usize, end: usize) -> Result<&[u8]> {
    if start >= end || end > buffer.len() {
        return Err(invalid_buffer());
    }
    let bytes = &buffer[start..end];
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "mapper field is not NUL-terminated"))?;
    Ok(&bytes[..nul])
}

/// Writes and zero-pads a fixed-size NUL-terminated ABI string field.
pub(super) fn write_c_string_fixed(
    buffer: &mut [u8],
    offset: usize,
    len: usize,
    value: impl AsRef<[u8]>,
) -> Result<()> {
    let value = value.as_ref();
    if value.len() >= len {
        return_errno_with_message!(Errno::EINVAL, "output string exceeds ABI field length");
    }
    require_range(buffer, offset, len)?;
    buffer[offset..offset + len].fill(0);
    buffer[offset..offset + value.len()].copy_from_slice(value);
    Ok(())
}

/// Writes a variable-length NUL-terminated ABI string.
pub(super) fn write_c_string(
    buffer: &mut [u8],
    offset: usize,
    value: impl AsRef<[u8]>,
) -> Result<()> {
    let value = value.as_ref();
    let len = value.len().checked_add(1).ok_or_else(invalid_buffer)?;
    require_range(buffer, offset, len)?;
    buffer[offset..offset + value.len()].copy_from_slice(value);
    buffer[offset + value.len()] = 0;
    Ok(())
}

/// Verifies an ABI field range is fully contained in the copied buffer.
pub(super) fn require_range(buffer: &[u8], offset: usize, len: usize) -> Result<()> {
    let end = offset.checked_add(len).ok_or_else(invalid_buffer)?;
    if end > buffer.len() {
        return Err(invalid_buffer());
    }
    Ok(())
}

/// Reads one little-endian `u32` from an aligned or unaligned ABI offset.
pub(super) fn read_u32(buffer: &[u8], offset: usize) -> Result<u32> {
    require_range(buffer, offset, 4)?;
    Ok(u32::from_le_bytes(
        buffer[offset..offset + 4].try_into().unwrap(),
    ))
}

/// Reads one little-endian `u64` from an aligned or unaligned ABI offset.
pub(super) fn read_u64(buffer: &[u8], offset: usize) -> Result<u64> {
    require_range(buffer, offset, 8)?;
    Ok(u64::from_le_bytes(
        buffer[offset..offset + 8].try_into().unwrap(),
    ))
}

/// Writes one little-endian `u32` value after range and conversion checks.
pub(super) fn write_u32(buffer: &mut [u8], offset: usize, value: impl TryInto<u32>) -> Result<()> {
    let value = value
        .try_into()
        .map_err(|_| Error::with_message(Errno::EINVAL, "value does not fit in a u32 ABI field"))?;
    require_range(buffer, offset, 4)?;
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Writes one little-endian `u64` value into the copied ABI buffer.
pub(super) fn write_u64(buffer: &mut [u8], offset: usize, value: u64) -> Result<()> {
    require_range(buffer, offset, 8)?;
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Decodes the low 32 bits, matching Linux's `huge_decode_dev` ioctl contract.
pub(super) fn decode_ioctl_device(raw: u64) -> DeviceId {
    let raw = raw as u32;
    let major = ((raw >> 8) & 0xfff) as u16;
    let minor = (raw & 0xff) | ((raw >> 12) & 0xfff00);
    DeviceId::new(MajorId::new(major), MinorId::new(minor))
}

pub(super) fn invalid_buffer() -> Error {
    Error::with_message(Errno::EINVAL, "invalid Device Mapper ioctl buffer layout")
}

/// Preserves the allocation-error boundary of Linux `dev_create`/`dm_create`.
/// Device-number allocation failures become ENXIO before identity insertion;
/// name and UUID collisions report EBUSY.
pub(super) fn map_create_error(error: DmError) -> Error {
    match error {
        DmError::NameExists | DmError::UuidExists => {
            Error::with_message(Errno::EBUSY, "mapper identity already exists")
        }
        DmError::MinorBusy | DmError::MinorExhausted => {
            Error::with_message(Errno::ENXIO, "cannot allocate mapper device number")
        }
        other => map_dm_error(other),
    }
}

/// Decodes the supported subset, ignoring ioctl direction and size bits.
pub(super) fn decode_command(raw: u32) -> Result<u8> {
    if (raw >> 8) & 0xff != DM_IOCTL_MAGIC {
        return_errno_with_message!(Errno::ENOTTY, "unknown Device Mapper ioctl type");
    }
    let command = raw as u8;
    match command {
        DM_VERSION_CMD | DM_LIST_DEVICES_CMD | DM_DEV_CREATE_CMD | DM_DEV_REMOVE_CMD
        | DM_DEV_STATUS_CMD => Ok(command),
        _ => return_errno_with_message!(Errno::ENOTTY, "unsupported Device Mapper ioctl command"),
    }
}

/// Normalizes output flags and terminates identity fields before dispatch.
///
/// Linux list filters are bounded byte prefixes, not necessarily UTF-8 strings.
pub(super) fn validate_ioctl_params(command: u8, buffer: &mut [u8]) -> Result<()> {
    let flags = read_u32(buffer, OFF_FLAGS)?
        & !(DM_BUFFER_FULL_FLAG
            | DM_UEVENT_GENERATED_FLAG
            | DM_SECURE_DATA_FLAG
            | DM_DATA_OUT_FLAG);
    write_u32(buffer, OFF_FLAGS, flags)?;
    if command == DM_LIST_DEVICES_CMD {
        return Ok(());
    }
    if command == DM_DEV_CREATE_CMD {
        if buffer[OFF_NAME] == 0 {
            return_errno_with_message!(Errno::EINVAL, "missing mapper name");
        }
    } else if buffer[OFF_NAME] != 0 && buffer[OFF_UUID] != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "name and UUID selectors are mutually exclusive"
        );
    }
    buffer[OFF_NAME + DM_NAME_LEN - 1] = 0;
    buffer[OFF_UUID + DM_UUID_LEN - 1] = 0;
    if command == DM_DEV_REMOVE_CMD && flags & DM_DEFERRED_REMOVE != 0 {
        return_errno_with_message!(Errno::EOPNOTSUPP, "deferred remove is not supported");
    }
    Ok(())
}

pub(super) fn map_dm_error(error: DmError) -> Error {
    match error {
        DmError::NameExists | DmError::UuidExists | DmError::MinorBusy => {
            Error::with_message(Errno::EEXIST, "mapper identity already exists")
        }
        DmError::DeviceNotFound => {
            Error::with_message(Errno::ENXIO, "mapper device does not exist")
        }
        DmError::MinorExhausted | DmError::MajorExhausted => {
            Error::with_message(Errno::ENOSPC, "mapper device numbers exhausted")
        }
    }
}
