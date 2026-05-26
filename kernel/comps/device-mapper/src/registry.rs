// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};

use aster_block::{BlockDevice, MajorIdOwner};
use device_id::{DeviceId, MinorId};
use ostd::sync::Mutex;
use spin::Once;

use crate::{DmDevice, DmError, table::DmTable};

const FIRST_MINOR: u32 = 0;

static DM_MAJOR_ID: Once<MajorIdOwner> = Once::new();
static DM_REGISTRY: Mutex<DmRegistry> = Mutex::new(DmRegistry::new());

#[derive(Debug)]
struct DmRegistry {
    next_minor: u32,
    devices_by_name: BTreeMap<String, Arc<DmDevice>>,
}

impl DmRegistry {
    const fn new() -> Self {
        Self {
            next_minor: FIRST_MINOR,
            devices_by_name: BTreeMap::new(),
        }
    }

    fn allocate_id(&mut self) -> Result<DeviceId, DmError> {
        let major = DM_MAJOR_ID.get().ok_or(DmError::NoDeviceId)?.get();
        let minor = self.next_minor;
        self.next_minor = self.next_minor.checked_add(1).ok_or(DmError::NoDeviceId)?;
        Ok(DeviceId::new(major, MinorId::new(minor)))
    }
}

pub fn init() -> Result<(), DmError> {
    let major = aster_block::allocate_major().map_err(|_| DmError::NoDeviceId)?;
    DM_MAJOR_ID.call_once(|| major);
    Ok(())
}

pub fn create_device(name: String, table: DmTable) -> Result<Arc<DmDevice>, DmError> {
    if name.is_empty() || name.contains('/') {
        return Err(DmError::InvalidArgument);
    }

    let mut registry = DM_REGISTRY.lock();
    if registry.devices_by_name.contains_key(&name) {
        return Err(DmError::DeviceExists);
    }

    let id = registry.allocate_id()?;
    let device = Arc::new(DmDevice::new(name.clone(), id, table));
    aster_block::register(device.clone()).map_err(|_| DmError::DeviceExists)?;
    registry.devices_by_name.insert(name, device.clone());
    Ok(device)
}

pub fn remove_device(name: &str) -> Result<Arc<DmDevice>, DmError> {
    let mut registry = DM_REGISTRY.lock();
    let device = registry
        .devices_by_name
        .remove(name)
        .ok_or(DmError::DeviceNotFound)?;
    let _ = aster_block::unregister(device.id());
    Ok(device)
}

pub fn lookup_device(name: &str) -> Option<Arc<DmDevice>> {
    DM_REGISTRY.lock().devices_by_name.get(name).cloned()
}

pub fn list_devices() -> Vec<Arc<DmDevice>> {
    DM_REGISTRY
        .lock()
        .devices_by_name
        .values()
        .cloned()
        .collect()
}

pub fn default_name(index: usize) -> String {
    alloc::format!("dm-{}", index)
}

pub fn normalize_name(raw: &str, fallback_index: usize) -> String {
    if raw == "-" || raw.is_empty() {
        default_name(fallback_index)
    } else {
        raw.to_string()
    }
}
