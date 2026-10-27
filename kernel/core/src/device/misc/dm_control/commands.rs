// SPDX-License-Identifier: MPL-2.0

//! Tableless DM command dispatch, identity selection, and response encoding.
//!
//! Status and list operations admit control references before taking snapshots.
//! Removal revalidates the exact instance under its lifecycle lock, detaches it,
//! and only then drains admitted references without holding that lock.

use core::ops::Deref;

use aster_device_mapper::{DmControlRef, DmDevice, DmManager};

use super::{
    abi::*,
    device_ops::{ControlWorkflow, CreateRequest, DeviceSnapshot, QueryWorkflow},
    manager,
};
use crate::prelude::*;

pub(super) fn handle_command(command: u8, buffer: &mut [u8]) -> Result<()> {
    match command {
        DM_DEV_CREATE_CMD => create_device(buffer),
        DM_DEV_REMOVE_CMD => remove_device(buffer),
        DM_DEV_STATUS_CMD => device_status(buffer),
        DM_LIST_DEVICES_CMD => list_devices(buffer),
        _ => unreachable!(),
    }
}

pub(super) fn create_device(buffer: &mut [u8]) -> Result<()> {
    let name = required_c_string(buffer, OFF_NAME, DM_NAME_LEN)?;
    let uuid = optional_uuid(buffer, OFF_UUID, DM_UUID_LEN)?;
    let flags = read_u32(buffer, OFF_FLAGS)?;
    let requested_minor = if flags & DM_PERSISTENT_DEV_FLAG != 0 {
        let dev = decode_ioctl_device(read_u64(buffer, OFF_DEV)?);
        Some(dev.minor().get())
    } else {
        None
    };

    validate_mapper_name(&name)?;
    let device = ControlWorkflow::new(manager()).create(CreateRequest {
        name,
        uuid,
        requested_minor,
    })?;
    fill_device_header(buffer, &device)
}

fn remove_device(buffer: &mut [u8]) -> Result<()> {
    // REMOVE must not hold a control reference to the device it will drain.
    let device = lookup_device_in_manager(buffer, manager())?;
    with_current_device_lifecycle_in(manager(), &device, |device| {
        // Linux dev_remove returns identity, not a synthetic final status. Other
        // input header fields (including dev and event_nr) remain unchanged.
        write_c_string_fixed(buffer, OFF_NAME, DM_NAME_LEN, device.mapper_name())?;
        write_c_string_fixed(
            buffer,
            OFF_UUID,
            DM_UUID_LEN,
            device.uuid().as_deref().unwrap_or(b""),
        )?;
        let flags = read_u32(buffer, OFF_FLAGS)? & !(DM_INACTIVE_PRESENT_FLAG | DM_DEFERRED_REMOVE);
        write_u32(buffer, OFF_FLAGS, flags)?;
        ControlWorkflow::new(manager()).remove(device)
    })?;
    device.drain_removed();
    Ok(())
}

fn lookup_device(buffer: &[u8]) -> Result<DmControlRef> {
    let device = lookup_device_in_manager(buffer, manager())?;
    manager()
        .acquire_control(&device)
        .ok_or_else(|| Error::new(Errno::ENXIO))
}

fn device_status(buffer: &mut [u8]) -> Result<()> {
    let device = lookup_device(buffer)?;
    fill_device_header(buffer, &device)
}

fn list_devices(buffer: &mut [u8]) -> Result<()> {
    let include_uuid = read_u32(buffer, OFF_FLAGS)? & DM_UUID_FLAG != 0;
    let devices = manager()
        .devices()
        .into_iter()
        .filter_map(|device| manager().acquire_control(&device))
        .map(|device| QueryWorkflow::device(&device));
    list_devices_for_snapshots(buffer, devices, include_uuid)
}

/// Encodes Linux name-list records with event fields regardless of UUID mode.
///
/// Both filters are bounded byte prefixes, not necessarily terminated UTF-8.
/// Capacity is checked for the whole filtered list. Empty output still requires
/// room for a dev sentinel but does not include that sentinel in the output size.
pub(super) fn list_devices_for_snapshots(
    buffer: &mut [u8],
    devices: impl IntoIterator<Item = DeviceSnapshot>,
    include_uuid: bool,
) -> Result<()> {
    let name_prefix = fixed_bytes(buffer, OFF_NAME, DM_NAME_LEN)?;
    let uuid_prefix = fixed_bytes(buffer, OFF_UUID, DM_UUID_LEN)?;
    let devices: Vec<_> = devices
        .into_iter()
        .filter(|device| {
            device.name.as_bytes().starts_with(name_prefix)
                && device
                    .uuid
                    .as_deref()
                    .unwrap_or(b"")
                    .starts_with(uuid_prefix)
        })
        .collect();
    let needed = devices.iter().try_fold(0usize, |total, device| {
        let uuid_len = if include_uuid {
            device.uuid.as_ref().map_or(0, |uuid| uuid.len() + 1)
        } else {
            0
        };
        let (_, len) = name_list_record_layout(device.name.len() + 1, uuid_len)?;
        total.checked_add(len).ok_or_else(invalid_buffer)
    })?;
    let start = result_start(buffer)?;
    if available_from(buffer, start) < needed.max(8) {
        return set_buffer_full(buffer);
    }
    buffer[start..start + needed.max(8)].fill(0);
    let mut cursor = start;
    let mut previous = None;
    for device in devices {
        let uuid_len = if include_uuid {
            device.uuid.as_ref().map_or(0, |uuid| uuid.len() + 1)
        } else {
            0
        };
        let (extension_offset, record_len) =
            name_list_record_layout(device.name.len() + 1, uuid_len)?;
        if let Some(previous) = previous {
            write_u32(buffer, previous + 8, cursor - previous)?;
        }
        write_u64(buffer, cursor, device.id.as_encoded_u64())?;
        write_u32(buffer, cursor + 8, 0)?;
        write_c_string(buffer, cursor + 12, &device.name)?;
        let extension = cursor + extension_offset;
        // A tableless mapper has not generated a device event.
        write_u32(buffer, extension, 0)?;
        let flags = if !include_uuid {
            0
        } else if device.uuid.is_some() {
            DM_NAME_LIST_FLAG_HAS_UUID
        } else {
            DM_NAME_LIST_FLAG_DOESNT_HAVE_UUID
        };
        write_u32(buffer, extension + 4, flags)?;
        if include_uuid && let Some(uuid) = device.uuid {
            write_c_string(buffer, extension + 8, &uuid)?;
        }
        previous = Some(cursor);
        cursor += record_len;
    }
    write_u32(buffer, OFF_DATA_SIZE, start + needed)
}

pub(super) fn lookup_device_in_manager(
    buffer: &[u8],
    manager: &DmManager,
) -> Result<Arc<DmDevice>> {
    let raw_dev = read_u64(buffer, OFF_DEV)?;
    let name = optional_c_string(buffer, OFF_NAME, DM_NAME_LEN)?;
    let uuid = optional_uuid(buffer, OFF_UUID, DM_UUID_LEN)?;

    if name.is_some() && uuid.is_some() {
        return_errno_with_message!(
            Errno::EINVAL,
            "name and UUID selectors are mutually exclusive"
        );
    }
    if raw_dev != 0 && (name.is_some() || uuid.is_some()) {
        return_errno_with_message!(Errno::ENXIO, "ambiguous mapper selector");
    }
    let device = if let Some(uuid) = uuid {
        manager.lookup_uuid(&uuid)
    } else if let Some(name) = name {
        manager.lookup_name(&name)
    } else if raw_dev != 0 {
        manager.lookup_id(decode_ioctl_device(raw_dev))
    } else {
        return_errno_with_message!(Errno::ENXIO, "missing mapper selector");
    };
    device.ok_or_else(|| Error::with_message(Errno::ENXIO, "mapper device does not exist"))
}

/// Proof that a mapper is current and its lifecycle lock remains held.
pub(super) struct CurrentDevice<'a> {
    device: &'a Arc<DmDevice>,
    _guard: MutexGuard<'a, ()>,
}

impl Deref for CurrentDevice<'_> {
    type Target = Arc<DmDevice>;

    fn deref(&self) -> &Self::Target {
        self.device
    }
}

pub(super) fn with_current_device_lifecycle_in<T>(
    manager: &DmManager,
    device: &Arc<DmDevice>,
    operation: impl FnOnce(&CurrentDevice<'_>) -> Result<T>,
) -> Result<T> {
    let lifecycle = device.lock_lifecycle();
    if !manager.is_current(device) {
        return_errno_with_message!(Errno::ENXIO, "mapper device does not exist");
    }
    operation(&CurrentDevice {
        device,
        _guard: lifecycle,
    })
}

pub(super) fn fill_device_header(buffer: &mut [u8], device: &DmDevice) -> Result<()> {
    let device = QueryWorkflow::device(device);
    let flags = read_u32(buffer, OFF_FLAGS)?
        & !(DM_READONLY_FLAG
            | DM_SUSPEND_FLAG
            | DM_ACTIVE_PRESENT_FLAG
            | DM_INACTIVE_PRESENT_FLAG
            | DM_INTERNAL_SUSPEND_FLAG);
    // Without tables or runtime publication, both active and inactive queries
    // report zero targets, opens, and events. READONLY belongs to a loaded table.
    write_u32(buffer, OFF_TARGET_COUNT, 0)?;
    write_u32(buffer, OFF_FLAGS, flags)?;
    write_u32(buffer, OFF_EVENT_NR, 0)?;
    write_u64(buffer, OFF_DEV, device.id.as_encoded_u64())?;
    write_u32(buffer, OFF_OPEN_COUNT, 0)?;
    write_c_string_fixed(buffer, OFF_NAME, DM_NAME_LEN, &device.name)?;
    write_c_string_fixed(
        buffer,
        OFF_UUID,
        DM_UUID_LEN,
        device.uuid.as_deref().unwrap_or(b""),
    )
}

/// Rejects names that cannot be published, before any identity mutation.
pub(super) fn validate_mapper_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() >= DM_NAME_LEN
        || name.contains('/')
        || matches!(name, "." | ".." | "control")
    {
        return_errno_with_message!(Errno::EINVAL, "invalid mapper name");
    }
    Ok(())
}
