// SPDX-License-Identifier: MPL-2.0

//! A subsystem for character devices (or char devices for short).

use core::ops::Range;

use device_id::{DeviceId, MajorId, MinorId};

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

/// A registered minor interval, independent of individual device nodes.
struct RegisteredRegion {
    end: u32,
    name: String,
}

const MINOR_COUNT: u32 = MinorId::MAX.get() + 1;

// Linux chrdev_show lists registrations, not individual cdevs. A major can
// therefore appear more than once, ordered by the first minor of each region.
static REGIONS: Mutex<BTreeMap<(u16, u32), RegisteredRegion>> = Mutex::new(BTreeMap::new());

/// Acquires an entire major and records its driver name until the owner drops.
pub(crate) fn acquire_major(major: MajorId, name: &str) -> Result<MajorIdOwner> {
    acquire_region(major, 0..MINOR_COUNT, name).map(MajorIdOwner)
}

/// Acquires a nonempty minor interval within one major.
///
/// Disjoint intervals may share a major, including when their names differ.
/// Overlapping reservations fail without changing the registry. This records
/// number ownership only; callers remain responsible for publishing devices.
///
/// Reference: Linux 6.6 (5078a3a23a1e), fs/char_dev.c:__register_chrdev_region.
pub(crate) fn acquire_region(
    major: MajorId,
    minors: Range<u32>,
    name: &str,
) -> Result<DeviceRegionOwner> {
    if major.get() > MAX_MAJOR || minors.start >= minors.end || minors.end > MINOR_COUNT {
        return_errno_with_message!(Errno::EINVAL, "invalid character device number range");
    }

    let mut regions = REGIONS.lock();
    if regions
        .range((major.get(), 0)..=(major.get(), minors.end - 1))
        .any(|(_, region)| region.end > minors.start)
    {
        return_errno_with_message!(Errno::EEXIST, "character device number range is occupied");
    }
    regions.insert(
        (major.get(), minors.start),
        RegisteredRegion {
            end: minors.end,
            name: name.to_string(),
        },
    );
    Ok(DeviceRegionOwner {
        major,
        first_minor: minors.start,
    })
}

/// Allocates an unused major and records its driver name.
///
/// Even a partially reserved major is skipped. The existing dynamic allocation
/// order is retained; the owner reserves the complete minor interval.
#[expect(dead_code)]
pub(crate) fn allocate_major(name: &str) -> Result<MajorIdOwner> {
    let mut regions = REGIONS.lock();
    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if regions
            .range((id, 0)..=(id, MinorId::MAX.get()))
            .next()
            .is_some()
        {
            continue;
        }
        regions.insert(
            (id, 0),
            RegisteredRegion {
                end: MINOR_COUNT,
                name: name.to_string(),
            },
        );
        return Ok(MajorIdOwner(DeviceRegionOwner {
            major: MajorId::new(id),
            first_minor: 0,
        }));
    }
    return_errno_with_message!(Errno::ENOSPC, "no more major IDs are available");
}

/// Returns registered names ordered by major and starting minor.
///
/// The snapshot owns its strings so procfs does not hold the registry lock while
/// writing to userspace. Multiple regions of one major remain separate rows.
pub(crate) fn major_devices() -> Vec<(u16, String)> {
    REGIONS
        .lock()
        .iter()
        .map(|((major, _), region)| (*major, region.name.clone()))
        .collect()
}

/// Owns a character device number interval until dropped.
pub(crate) struct DeviceRegionOwner {
    major: MajorId,
    first_minor: u32,
}

impl Drop for DeviceRegionOwner {
    fn drop(&mut self) {
        REGIONS.lock().remove(&(self.major.get(), self.first_minor));
    }
}

/// Owns every minor of a character device major until dropped.
pub(crate) struct MajorIdOwner(DeviceRegionOwner);

impl MajorIdOwner {
    /// Returns the reserved major ID.
    pub(crate) fn get(&self) -> MajorId {
        self.0.major
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn disjoint_regions_share_a_major_and_release_independently() {
        let major = MajorId::new(509);
        let second = acquire_region(major, 8..16, "second").unwrap();
        let first = acquire_region(major, 0..8, "first").unwrap();
        let rows = || {
            major_devices()
                .into_iter()
                .filter(|(id, _)| *id == major.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(),
            vec![(509, "first".to_string()), (509, "second".to_string())]
        );
        for range in [0..1, 7..9, 8..16, 0..17] {
            assert_eq!(
                acquire_region(major, range, "overlap")
                    .err()
                    .unwrap()
                    .error(),
                Errno::EEXIST
            );
        }
        assert!(acquire_major(major, "whole").is_err());
        drop(first);
        assert_eq!(rows(), vec![(509, "second".to_string())]);
        let replacement = acquire_region(major, 0..8, "replacement").unwrap();
        drop(second);
        drop(replacement);
        assert!(rows().is_empty());
        let whole = acquire_major(major, "whole").unwrap();
        assert_eq!(whole.get(), major);
        assert!(acquire_region(major, 0..1, "conflict").is_err());
    }

    #[ktest]
    fn invalid_regions_do_not_reserve_numbers() {
        let major = MajorId::new(508);
        for (start, end) in [(0, 0), (4, 3), (0, MINOR_COUNT + 1)] {
            let range = start..end;
            assert_eq!(
                acquire_region(major, range, "invalid")
                    .err()
                    .unwrap()
                    .error(),
                Errno::EINVAL
            );
        }
        assert_eq!(
            acquire_major(MajorId::new(MAX_MAJOR + 1), "invalid")
                .err()
                .unwrap()
                .error(),
            Errno::EINVAL
        );
        let last = acquire_region(major, MinorId::MAX.get()..MINOR_COUNT, "last").unwrap();
        drop(last);
        let _whole = acquire_major(major, "whole").unwrap();
    }
}
