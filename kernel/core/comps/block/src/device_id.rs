// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::btree_map::{BTreeMap, Entry},
    vec::Vec,
};

use device_id::{DeviceId, MajorId, MinorId};
use id_alloc::IdAlloc;
use ostd::sync::Mutex;
use spin::Once;

use crate::Error;

/// The maximum value of the major device ID of a block device.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/block/genhd.c#L239>.
const MAX_MAJOR: u16 = 511;

/// Block devices that request a dynamic allocation of major ID will
/// take numbers starting from 254 and downward.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/block/genhd.c#L224>.
const LAST_DYNAMIC_MAJOR: u16 = 254;

static MAJORS: Mutex<BTreeMap<u16, &'static str>> = Mutex::new(BTreeMap::new());

/// Acquires a major ID with a name.
///
/// The name is attached to this major number registration. For example,
/// the name "virtblk" is attached to the major ID of virtio-blk devices and
/// the name "nvme" is attached to that of NVMe devices. These names along
/// with the major numbers are shown in `/proc/devices`.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn acquire_major(major: MajorId, name: &'static str) -> Result<MajorIdOwner, Error> {
    if major.get() > MAX_MAJOR {
        return Err(Error::InvalidArgs);
    }

    let mut majors = MAJORS.lock();
    match majors.entry(major.get()) {
        Entry::Occupied(_) => return Err(Error::IdAcquired),
        Entry::Vacant(entry) => entry.insert(name),
    };

    Ok(MajorIdOwner(major))
}

/// Allocates a major ID with a name.
///
/// The name is shown in `/proc/devices`.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn allocate_major(name: &'static str) -> Result<MajorIdOwner, Error> {
    let mut majors = MAJORS.lock();
    for id in (1..LAST_DYNAMIC_MAJOR + 1).rev() {
        if let Entry::Vacant(entry) = majors.entry(id) {
            entry.insert(name);
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    Err(Error::IdExhausted)
}

/// Collects all acquired major IDs and their names.
pub fn collect_major_devices() -> Vec<(u16, &'static str)> {
    MAJORS
        .lock()
        .iter()
        .map(|(major, name)| (*major, *name))
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

/// The major ID used for extended partitions when the number of disk partitions exceeds the standard limit.
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
            major: acquire_major(major, "blkext").unwrap(),
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

pub(super) fn init() {
    EXTENDED_DEVICE_ID_ALLOCATOR.call_once(ExtendedDeviceIdAllocator::new);
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn acquire_and_release_major() {
        // Use a major ID outside the dynamic allocation range to avoid
        // conflicting with majors allocated by other tests.
        let major = MajorId::new(300);

        let owner = acquire_major(major, "ktest").unwrap();
        assert_eq!(owner.get(), major);

        // An acquired major ID cannot be acquired again.
        assert!(acquire_major(major, "ktest2").is_err());

        // The name is shown in `collect_major_devices`.
        assert!(collect_major_devices().contains(&(300, "ktest")));

        // Once the owner is dropped, the major ID is released.
        drop(owner);
        assert!(!collect_major_devices().iter().any(|(id, _)| *id == 300));
    }

    #[ktest]
    fn allocate_major_in_dynamic_range() {
        let owner = allocate_major("ktest").unwrap();
        let major = owner.get().get();

        // The allocated major ID is in the dynamic allocation range.
        assert!((1..=LAST_DYNAMIC_MAJOR).contains(&major));
        assert!(collect_major_devices().contains(&(major, "ktest")));
    }
}
