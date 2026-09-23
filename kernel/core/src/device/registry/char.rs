// SPDX-License-Identifier: MPL-2.0

//! A subsystem for character devices (or char devices for short).

use alloc::collections::btree_map::Entry;
use core::ops::Range;

use device_id::{DeviceId, MajorId, MajorIdOwner};

use crate::{
    device::Device,
    fs::devtmpfs::{self, DevtmpfsNode},
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
    registry.insert(id, device.clone());

    if let Some(meta) = device.devtmpfs_meta()
        && let Err(error) =
            devtmpfs::create_node(DevtmpfsNode::new(device.type_(), device.id(), meta))
    {
        registry.remove(&id);
        return Err(error);
    }
    Ok(())
}

/// Unregisters an existing char device, returning the device if found.
pub fn unregister(id: DeviceId) -> Result<Arc<dyn Device>> {
    let mut registry = DEVICE_REGISTRY.lock();
    let device = registry
        .remove(&id.to_raw())
        .ok_or_else(|| Error::with_message(Errno::ENOENT, "the char device does not exist"))?;

    if let Some(meta) = device.devtmpfs_meta()
        && let Err(error) =
            devtmpfs::delete_node(DevtmpfsNode::new(device.type_(), device.id(), meta))
    {
        warn!(
            "failed to delete devtmpfs node for char device {:?}: {:?}",
            id, error
        );
    }
    Ok(device)
}

/// Looks up a char device of a given device ID.
pub(super) fn lookup(id: DeviceId) -> Option<Arc<dyn Device>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

/// The maximum value of the major device ID of a char device.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/fs/char_dev.c#L104>.
pub(crate) const MAX_MAJOR: u16 = 511;

/// The ranges of free char majors.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/linux/fs.h#L2840>.
const DYNAMIC_MAJOR_ID_RANGES: [Range<u16>; 2] = [234..255, 384..512];

static MAJORS: Mutex<BTreeMap<u16, Vec<&'static str>>> = Mutex::new(BTreeMap::new());

/// Acquires a major ID with a name.
///
/// The name is attached to this major number registration. For example,
/// the name "mem" is attached to the major ID 1 of memory devices such as
/// `/dev/null` and `/dev/zero`.
///
/// The returned `MajorIdOwner` object represents the ownership to the major ID.
/// Until the object is dropped, this major ID cannot be acquired via `acquire_major` or `allocate_major` again.
pub fn acquire_major(major: MajorId, name: &'static str) -> Result<MajorIdOwner> {
    if major.get() > MAX_MAJOR {
        return_errno_with_message!(Errno::EINVAL, "the major ID is invalid");
    }

    let mut majors = MAJORS.lock();

    // A single major ID may be registered under multiple names (e.g., major 4
    // is shared by `tty`, `ttyS`, and `/dev/vc/0` in Linux). We therefore allow
    // repeated acquisitions of the same major as long as the name is new. This
    // mirrors Linux's `chrdevs[]` hash buckets, where each (major, name) pair
    // is an independent node on the same bucket's list.
    let names = majors.entry(major.get()).or_default();
    if names.contains(&name) {
        return_errno_with_message!(
            Errno::EEXIST,
            "the (major, name) pair has already been acquired"
        );
    }
    names.push(name);

    Ok(MajorIdOwner::new(major, name, release_major))
}

/// Allocates a major ID with a name.
///
/// Similar to [`acquire_major`], this function returns a free major ID.
/// The difference is that this function allocates the largest free major ID,
/// rather than a specified one.
#[expect(dead_code)]
pub(crate) fn allocate_major(name: &'static str) -> Result<MajorIdOwner> {
    let mut majors = MAJORS.lock();

    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if let Entry::Vacant(entry) = majors.entry(id) {
            entry.insert(Vec::from([name]));
            return Ok(MajorIdOwner::new(MajorId::new(id), name, release_major));
        }
    }

    return_errno_with_message!(Errno::ENOSPC, "no more major IDs are available");
}

/// Collects all acquired major IDs and their names.
pub(crate) fn collect_major_devices() -> Vec<(u16, &'static str)> {
    MAJORS
        .lock()
        .iter()
        .flat_map(|(major, names)| names.iter().map(|name| (*major, *name)))
        .collect()
}

/// Releases a major ID, removing it from the registry.
///
/// This is the release function used by [`MajorIdOwner`]s created in this module.
fn release_major(major: u16, name: &'static str) {
    let mut majors = MAJORS.lock();
    if let Some(names) = majors.get_mut(&major) {
        names.retain(|n| *n != name);
        if names.is_empty() {
            majors.remove(&major);
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn acquire_and_release_major() {
        // Use a major ID outside the dynamic allocation ranges to avoid
        // conflicting with majors allocated by other tests.
        let major = MajorId::new(42);

        let owner = acquire_major(major, "ktest").unwrap();
        assert_eq!(owner.get(), major);

        // The same major can be acquired again with a different name.
        let owner2 = acquire_major(major, "ktest2").unwrap();
        assert!(acquire_major(major, "ktest2").is_err());

        // Both names are shown in `collect_major_devices`.
        assert!(collect_major_devices().contains(&(42, "ktest")));
        assert!(collect_major_devices().contains(&(42, "ktest2")));

        // Dropping only one owner keeps the other name registered.
        drop(owner);
        assert!(!collect_major_devices().contains(&(42, "ktest")));
        assert!(collect_major_devices().contains(&(42, "ktest2")));

        // Once the second owner is dropped, the major ID is fully released.
        drop(owner2);
        assert!(!collect_major_devices().iter().any(|(id, _)| *id == 42));
    }

    #[ktest]
    fn acquire_invalid_major() {
        assert!(acquire_major(MajorId::new(MAX_MAJOR + 1), "ktest").is_err());
    }
}
