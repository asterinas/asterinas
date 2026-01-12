// SPDX-License-Identifier: MPL-2.0

//! A subsystem for character devices (or char devices for short).

use core::ops::Range;

use device_id::{DeviceId, MajorId};

use crate::{
    fs::{
        device::{Device, DeviceType, add_node},
        path::PathResolver,
    },
    prelude::*,
};

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn Device>>> = Mutex::new(BTreeMap::new());

/// Registers a new char device.
pub fn register(device: Arc<dyn Device>) -> Result<()> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return_errno_with_message!(Errno::EEXIST, "the char device already exists");
    }
    registry.insert(id, device);

    Ok(())
}

/// Unregisters an existing char device, returning the device if found.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn Device>> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "the char device does not exist"))
}

/// Collects all char devices.
pub fn collect_all() -> Vec<Arc<dyn Device>> {
    DEVICE_REGISTRY.lock().values().cloned().collect()
}

/// Looks up a char device of a given device ID.
pub(super) fn lookup(id: DeviceId) -> Option<Arc<dyn Device>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

/// The maximum value of the major device ID of a char device.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/fs/char_dev.c#L104>.
pub const MAX_MAJOR: u16 = 511;

/// The ranges of free char majors.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/fs.h#L2840>.
const DYNAMIC_MAJOR_ID_RANGES: [Range<u16>; 2] = [234..255, 384..512];

static MAJORS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// Acquires a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn acquire_major(major: MajorId) -> Result<MajorIdOwner> {
    if major.get() > MAX_MAJOR {
        return_errno_with_message!(Errno::EINVAL, "the major ID is invalid");
    }

    if MAJORS.lock().insert(major.get()) {
        Ok(MajorIdOwner(major))
    } else {
        return_errno_with_message!(Errno::EEXIST, "the major ID has already been acquired")
    }
}

/// Allocates a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
#[expect(dead_code)]
pub fn allocate_major() -> Result<MajorIdOwner> {
    let mut majors = MAJORS.lock();

    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if majors.insert(id) {
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    return_errno_with_message!(Errno::ENOSPC, "no more major IDs are available");
}

/// An owned major ID.
///
/// Each instances of this type will unregister the major ID when dropped.
pub struct MajorIdOwner(MajorId);

impl MajorIdOwner {
    /// Returns the major ID.
    pub fn get(&self) -> MajorId {
        self.0
    }
}

impl Drop for MajorIdOwner {
    fn drop(&mut self) {
        MAJORS.lock().remove(&self.0.get());
    }
}

pub(super) fn init_in_first_process(path_resolver: &PathResolver) -> Result<()> {
    for device in collect_all() {
        if let Some(devtmpfs_path) = device.devtmpfs_path() {
            let dev_id = device.id().as_encoded_u64();
            add_node(DeviceType::Char, dev_id, &devtmpfs_path, path_resolver)?;
        }
    }

    Ok(())
}
