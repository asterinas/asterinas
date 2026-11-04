// SPDX-License-Identifier: MPL-2.0

//! Linux DM command decoding and version-field access.
//!
//! Helpers operate only on copied byte slices; userspace access and ioctl
//! ordering remain in the control-device entry point.

use crate::prelude::*;

pub(super) const DM_IOCTL_MAGIC: u32 = 0xfd;
pub(super) const DM_VERSION_CMD: u8 = 0;
pub(super) const DM_VERSION: [u32; 3] = [4, 48, 0];

const OFF_VERSION: usize = 0;

/// Decodes the Linux ioctl number and rejects command IDs outside this DM subset.
pub(super) fn decode_command(raw: u32) -> Result<u8> {
    if (raw >> 8) & 0xff != DM_IOCTL_MAGIC {
        return_errno_with_message!(Errno::ENOTTY, "unknown Device Mapper ioctl type");
    }
    let command = raw as u8;
    match command {
        DM_VERSION_CMD => Ok(command),
        _ => return_errno_with_message!(Errno::ENOTTY, "unsupported Device Mapper ioctl command"),
    }
}

/// Accepts the same major and a minor no newer than this implementation's ABI.
///
/// Linux `check_version` ignores patch differences. The caller must report the
/// kernel version to userspace even when this check rejects the client version.
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

/// Writes the server version into the copied version fields.
pub(super) fn write_version(buffer: &mut [u8]) -> Result<()> {
    for (index, value) in DM_VERSION.into_iter().enumerate() {
        write_u32(buffer, OFF_VERSION + index * 4, value)?;
    }
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

/// Writes one little-endian `u32` value after range and conversion checks.
pub(super) fn write_u32(buffer: &mut [u8], offset: usize, value: impl TryInto<u32>) -> Result<()> {
    let value = value
        .try_into()
        .map_err(|_| Error::with_message(Errno::EINVAL, "value does not fit in a u32 ABI field"))?;
    require_range(buffer, offset, 4)?;
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Returns the error for a field outside the copied ABI buffer.
pub(super) fn invalid_buffer() -> Error {
    Error::with_message(Errno::EINVAL, "invalid Device Mapper ioctl buffer layout")
}
