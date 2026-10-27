// SPDX-License-Identifier: MPL-2.0

//! Block major ownership and driver names exported through `/proc/devices`.

use alloc::{collections::BTreeMap, string::String};

use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

use crate::Error;

/// The maximum value of the major device ID of a block device.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/block/genhd.c#L239>.
pub const MAX_MAJOR: u16 = 511;

/// Block devices that request a dynamic allocation of major ID will
/// take numbers starting from 254 and downward.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/block/genhd.c#L224>.
const LAST_DYNAMIC_MAJOR: u16 = 254;

static MAJORS: Mutex<BTreeMap<u16, String>> = Mutex::new(BTreeMap::new());

/// Acquires a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via
/// `acquire_major` or `allocate_major` again.
pub fn acquire_major(major: MajorId) -> Result<MajorIdOwner, Error> {
    acquire_major_with_name(major, "unknown")
}

/// Acquires a major ID and records its stable driver name for `/proc/devices`.
pub fn acquire_major_with_name(major: MajorId, name: &str) -> Result<MajorIdOwner, Error> {
    if major.get() > MAX_MAJOR {
        return Err(Error::InvalidArgs);
    }

    let mut majors = MAJORS.lock();
    if majors.contains_key(&major.get()) {
        return Err(Error::IdAcquired);
    }
    majors.insert(major.get(), String::from(name));
    Ok(MajorIdOwner(major))
}

/// Allocates a major ID.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via
/// `acquire_major` or `allocate_major` again.
pub fn allocate_major() -> Result<MajorIdOwner, Error> {
    allocate_major_with_name("unknown")
}

/// Allocates a dynamic major ID and records its stable driver name for `/proc/devices`.
pub fn allocate_major_with_name(name: &str) -> Result<MajorIdOwner, Error> {
    let mut majors = MAJORS.lock();
    for id in (1..LAST_DYNAMIC_MAJOR + 1).rev() {
        if let alloc::collections::btree_map::Entry::Vacant(entry) = majors.entry(id) {
            entry.insert(String::from(name));
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    Err(Error::IdExhausted)
}

/// Returns a sorted snapshot of the block major ownership registry.
pub fn major_devices() -> alloc::vec::Vec<(u16, String)> {
    MAJORS
        .lock()
        .iter()
        .map(|(major, name)| (*major, name.clone()))
        .collect()
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

/// The major ID used for extended partitions when the number of disk partitions
/// exceeds the standard limit.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/block/partitions/core.c#L352>.
const EXTENDED_MAJOR: u16 = 259;

/// An allocator for extended device IDs.
pub(crate) struct ExtendedDeviceIdAllocator {
    major: MajorIdOwner,
    minor_allocator: Mutex<IdAlloc>,
}

impl ExtendedDeviceIdAllocator {
    fn new() -> Self {
        let major = MajorId::new(EXTENDED_MAJOR);
        let minor_allocator = IdAlloc::with_capacity(MinorId::MAX.get() as usize + 1);

        Self {
            major: acquire_major_with_name(major, "blkext").unwrap(),
            minor_allocator: Mutex::new(minor_allocator),
        }
    }

    /// Allocates an extended device ID.
    pub(crate) fn allocate(&self) -> DeviceId {
        let minor = self.minor_allocator.lock().alloc().unwrap() as u32;

        DeviceId::new(self.major.get(), MinorId::new(minor))
    }

    /// Releases an extended device ID.
    #[expect(dead_code)]
    pub(crate) fn release(&mut self, id: DeviceId) {
        if id.major() != self.major.get() {
            return;
        }

        self.minor_allocator.lock().free(id.minor().get() as usize);
    }
}

pub(crate) static EXTENDED_DEVICE_ID_ALLOCATOR: Once<ExtendedDeviceIdAllocator> = Once::new();

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn major_snapshot_is_sorted_and_tracks_owner_lifetime() {
        let before = major_devices();
        let first = acquire_major_with_name(MajorId::new(508), "first-major").unwrap();
        let second = acquire_major_with_name(MajorId::new(507), "second-major").unwrap();

        let snapshot = major_devices();
        let first_index = snapshot
            .iter()
            .position(|(major, _)| *major == 507)
            .unwrap();
        let second_index = snapshot
            .iter()
            .position(|(major, _)| *major == 508)
            .unwrap();
        assert!(first_index < second_index);
        assert_eq!(snapshot[first_index].1, "second-major");
        assert_eq!(snapshot[second_index].1, "first-major");

        drop(first);
        assert!(major_devices().iter().all(|(major, _)| *major != 508));
        drop(second);
        assert_eq!(major_devices(), before);
    }
}

pub(super) fn init() {
    EXTENDED_DEVICE_ID_ALLOCATOR.call_once(ExtendedDeviceIdAllocator::new);
}
