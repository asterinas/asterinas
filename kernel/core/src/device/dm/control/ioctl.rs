// SPDX-License-Identifier: MPL-2.0

//! Common copy, validation, secure wiping, and copyout for DM control commands.

use ostd::mm::VmIo;

use super::abi;
use crate::{context::current_userspace, prelude::*};

/// Owns copied ioctl input and wipes the full payload of secure requests.
struct IoctlBuffer {
    bytes: Vec<u8>,
    secure: bool,
}

impl Drop for IoctlBuffer {
    fn drop(&mut self) {
        if self.secure {
            let len = self.bytes.len();
            VmWriter::from(self.bytes.as_mut_slice()).fill_zeros(len);
        }
    }
}

/// Copies a bounded LOAD payload, or a fixed prefix for commands without input records.
/// Query commands allocate output capacity without reading it from userspace.
pub(super) fn process_ioctl_parameters(
    arg: usize,
    version: &[u8; 12],
    command: u8,
    dispatch: impl FnOnce(&mut [u8]) -> Result<()>,
) -> Result<i32> {
    let mut header = [0u8; abi::FIXED_PREFIX_SIZE];
    header[..version.len()].copy_from_slice(version);
    let addr = arg
        .checked_add(version.len())
        .ok_or_else(|| errno!(EFAULT, "device-mapper ioctl address overflow"))?;
    current_userspace!().read_bytes(addr, &mut header[version.len()..])?;
    let input_size = abi::read_u32(&header, abi::OFF_DATA_SIZE);
    abi::validate_ioctl_size(input_size)?;
    let variable = matches!(
        command,
        abi::DM_TABLE_LOAD_CMD
            | abi::DM_LIST_DEVICES_CMD
            | abi::DM_TABLE_STATUS_CMD
            | abi::DM_TABLE_DEPS_CMD
            | abi::DM_LIST_VERSIONS_CMD
            | abi::DM_GET_TARGET_VERSION_CMD
    );
    let size = if variable {
        input_size as usize
    } else {
        debug_assert!(matches!(
            command,
            abi::DM_DEV_CREATE_CMD
                | abi::DM_DEV_REMOVE_CMD
                | abi::DM_DEV_SUSPEND_CMD
                | abi::DM_DEV_STATUS_CMD
                | abi::DM_TABLE_CLEAR_CMD
        ));
        abi::FIXED_PREFIX_SIZE
    };
    let secure = abi::read_u32(&header, abi::OFF_FLAGS) & abi::SECURE_DATA_FLAG != 0;
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(size).is_err() {
        if secure {
            current_userspace!()
                .writer(arg, size)?
                .fill_zeros(size)
                .map_err(|(error, _)| error)?;
        }
        return Err(errno!(ENOMEM, "cannot allocate mapper ioctl buffer"));
    }
    bytes.resize(size, 0);
    let mut buffer = IoctlBuffer { bytes, secure };
    buffer.bytes[..header.len()].copy_from_slice(&header);
    if size > abi::FIXED_PREFIX_SIZE && command == abi::DM_TABLE_LOAD_CMD {
        let payload_addr = arg
            .checked_add(abi::FIXED_PREFIX_SIZE)
            .ok_or_else(|| errno!(EFAULT, "device-mapper ioctl address overflow"))?;
        current_userspace!()
            .read_bytes(payload_addr, &mut buffer.bytes[abi::FIXED_PREFIX_SIZE..])?;
    }
    if secure {
        current_userspace!()
            .writer(arg, size)?
            .fill_zeros(size)
            .map_err(|(error, _)| error)?;
    }
    abi::validate_ioctl_params(
        buffer.bytes[..abi::FIXED_PREFIX_SIZE]
            .as_mut()
            .try_into()
            .unwrap(),
        command,
    )?;
    abi::write_u32(
        &mut buffer.bytes,
        abi::OFF_DATA_SIZE,
        abi::FIXED_PREFIX_SIZE as u32,
    );
    dispatch(&mut buffer.bytes)?;
    let output_len = output_len(command, variable, &buffer.bytes);
    current_userspace!().write_bytes(arg, &buffer.bytes[..output_len])?;
    Ok(0)
}

fn output_len(command: u8, variable: bool, bytes: &[u8]) -> usize {
    if variable && command != abi::DM_TABLE_LOAD_CMD {
        (abi::read_u32(bytes, abi::OFF_DATA_SIZE) as usize)
            .max(abi::FIXED_PREFIX_SIZE)
            .min(bytes.len())
    } else {
        abi::FIXED_PREFIX_SIZE
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn query_copyout_respects_capacity_and_reported_size() {
        for command in [
            abi::DM_LIST_DEVICES_CMD,
            abi::DM_TABLE_STATUS_CMD,
            abi::DM_TABLE_DEPS_CMD,
            abi::DM_LIST_VERSIONS_CMD,
            abi::DM_GET_TARGET_VERSION_CMD,
        ] {
            for (capacity, reported, expected) in [
                (abi::FIXED_PREFIX_SIZE, 312, abi::FIXED_PREFIX_SIZE),
                (312, 312, 312),
                (320, 319, 319),
                (320, 321, 320),
            ] {
                let mut bytes = vec![0; capacity];
                abi::write_u32(&mut bytes, abi::OFF_DATA_SIZE, reported as u32);
                assert_eq!(output_len(command, true, &bytes), expected);
            }
        }
        let mut load = vec![0; 512];
        abi::write_u32(&mut load, abi::OFF_DATA_SIZE, 512);
        assert_eq!(
            output_len(abi::DM_TABLE_CLEAR_CMD, false, &load),
            abi::FIXED_PREFIX_SIZE
        );
        assert_eq!(
            output_len(abi::DM_TABLE_LOAD_CMD, true, &load),
            abi::FIXED_PREFIX_SIZE
        );
        let mut info = vec![0; abi::FIXED_PREFIX_SIZE];
        abi::write_u32(&mut info, abi::OFF_DATA_SIZE, 312);
        assert_eq!(
            output_len(abi::DM_DEV_STATUS_CMD, false, &info),
            abi::FIXED_PREFIX_SIZE
        );
    }
}
