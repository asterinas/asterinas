// SPDX-License-Identifier: MPL-2.0

//! A subsystem for character devices (or char devices for short).
//!
//! Number reservations do not register device objects or create device nodes.

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
        return Err(errno!(EEXIST, "the char device already exists"));
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
        .ok_or_else(|| errno!(ENOENT, "the char device does not exist"))?;

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

const MINOR_COUNT: u32 = MinorId::MAX.get() + 1;

// Named minor reservations, ordered by major and starting minor.
// Regions within a major never overlap.
static REGIONS: Mutex<BTreeMap<RegionKey, RegisteredRegion>> = Mutex::new(BTreeMap::new());

/// Acquires an entire major and associates a registration name with it.
///
/// The major remains reserved until the returned owner is dropped.
pub fn acquire_major(major: MajorId, name: &str) -> Result<MajorIdOwner> {
    reserve_region(major, 0..MINOR_COUNT, name)?;
    Ok(MajorIdOwner(major))
}

/// Acquires a nonempty minor interval within one major.
///
/// Disjoint intervals may share a major, including when their names differ.
/// Overlapping reservations fail without changing the registry.
pub(crate) fn acquire_region(
    major: MajorId,
    minors: Range<u32>,
    name: &str,
) -> Result<DeviceRegionOwner> {
    reserve_region(major, minors.clone(), name)?;
    Ok(DeviceRegionOwner {
        major,
        first_minor: minors.start,
    })
}

/// Acquires a single minor and associates a registration name with it.
pub(crate) fn acquire_minor(
    major: MajorId,
    minor: MinorId,
    name: &str,
) -> Result<DeviceRegionOwner> {
    acquire_region(major, minor.get()..minor.get() + 1, name)
}

fn reserve_region(major: MajorId, minors: Range<u32>, name: &str) -> Result<()> {
    if major.get() > MAX_MAJOR || minors.start >= minors.end || minors.end > MINOR_COUNT {
        return Err(errno!(EINVAL, "invalid character device number range"));
    }

    let mut regions = REGIONS.lock();
    if regions
        .range(
            RegionKey {
                major: major.get(),
                first_minor: 0,
            }..=RegionKey {
                major: major.get(),
                first_minor: minors.end - 1,
            },
        )
        .next_back()
        .is_some_and(|(_, region)| region.end_minor_exclusive > minors.start)
    {
        return Err(errno!(EEXIST, "character device number range is occupied"));
    }
    regions.insert(
        RegionKey {
            major: major.get(),
            first_minor: minors.start,
        },
        RegisteredRegion {
            end_minor_exclusive: minors.end,
            name: name.to_string(),
        },
    );
    Ok(())
}

/// Allocates an unused major and associates a registration name with it.
///
/// Even a partially reserved major is skipped.
/// The owner reserves the complete minor interval.
#[cfg_attr(not(ktest), expect(dead_code))]
pub(crate) fn allocate_major(name: &str) -> Result<MajorIdOwner> {
    let mut regions = REGIONS.lock();
    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if regions
            .range(
                RegionKey {
                    major: id,
                    first_minor: 0,
                }..=RegionKey {
                    major: id,
                    first_minor: MinorId::MAX.get(),
                },
            )
            .next()
            .is_some()
        {
            continue;
        }
        regions.insert(
            RegionKey {
                major: id,
                first_minor: 0,
            },
            RegisteredRegion {
                end_minor_exclusive: MINOR_COUNT,
                name: name.to_string(),
            },
        );
        return Ok(MajorIdOwner(MajorId::new(id)));
    }
    Err(errno!(ENOSPC, "no more major IDs are available"))
}

/// Returns a snapshot of `(major_number, registration_name)` pairs,
/// sorted by major number and starting minor in ascending order.
///
/// Each reserved interval contributes one entry.
/// Entries with the same major remain separate even if their names match.
pub(crate) fn major_devices() -> Vec<(u16, String)> {
    REGIONS
        .lock()
        .iter()
        .map(|(key, region)| (key.major, region.name.clone()))
        .collect()
}

/// An owned character device number interval.
///
/// Dropping the owner releases the number reservation.
pub(crate) struct DeviceRegionOwner {
    major: MajorId,
    first_minor: u32,
}

impl Drop for DeviceRegionOwner {
    fn drop(&mut self) {
        REGIONS.lock().remove(&RegionKey {
            major: self.major.get(),
            first_minor: self.first_minor,
        });
    }
}

/// An owned character device major, including all its minors.
///
/// Dropping the owner releases the number reservation.
pub struct MajorIdOwner(MajorId);

impl MajorIdOwner {
    /// Returns the reserved major ID.
    pub(crate) fn get(&self) -> MajorId {
        self.0
    }
}

impl Drop for MajorIdOwner {
    fn drop(&mut self) {
        REGIONS.lock().remove(&RegionKey {
            major: self.0.get(),
            first_minor: 0,
        });
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct RegionKey {
    major: u16,
    first_minor: u32,
}

/// A registered minor interval, independent of individual device nodes.
struct RegisteredRegion {
    end_minor_exclusive: u32,
    name: String,
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
        let rows_fn = || {
            major_devices()
                .into_iter()
                .filter(|(id, _)| *id == major.get())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows_fn(),
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
        assert_eq!(rows_fn(), vec![(509, "second".to_string())]);
        let replacement = acquire_region(major, 0..8, "replacement").unwrap();
        drop(second);
        drop(replacement);
        assert!(rows_fn().is_empty());
        let whole = acquire_major(major, "whole").unwrap();
        assert_eq!(whole.get(), major);
        assert!(acquire_region(major, 0..1, "conflict").is_err());
    }

    #[ktest]
    fn minor_boundaries_and_major_owners_release_reservations() {
        let major = MajorId::new(507);
        let last = acquire_minor(major, MinorId::MAX, "last").unwrap();
        let first = acquire_minor(major, MinorId::new(0), "first").unwrap();
        assert!(acquire_major(major, "whole").is_err());
        assert!(acquire_minor(major, MinorId::MAX, "overlap").is_err());
        drop(last);
        drop(first);
        let whole = acquire_major(major, "whole").unwrap();
        assert!(acquire_minor(major, MinorId::MAX, "overlap").is_err());
        assert!(acquire_major(major, "overlap").is_err());
        drop(whole);
        let _replacement = acquire_major(major, "replacement").unwrap();
    }

    #[ktest]
    fn interval_neighbors_and_distinct_majors_are_independent() {
        let major = MajorId::new(505);
        let middle = acquire_region(major, 8..16, "same").unwrap();
        let last = acquire_region(major, 24..32, "same").unwrap();
        let first = acquire_region(major, 0..8, "same").unwrap();
        let gap = acquire_region(major, 16..24, "same").unwrap();
        let other = acquire_region(MajorId::new(504), 0..32, "other").unwrap();
        for range in [1..7, 4..28, 7..9, 23..25] {
            assert!(acquire_region(major, range, "overlap").is_err());
        }
        assert_eq!(
            major_devices()
                .iter()
                .filter(|(id, _)| *id == major.get())
                .count(),
            4
        );
        drop((first, middle, gap, last, other));
        let _whole = acquire_major(major, "whole").unwrap();
    }

    #[ktest]
    fn dynamic_major_owner_reserves_and_releases_all_minors() {
        let owner = allocate_major("dynamic").unwrap();
        let major = owner.get();
        assert!(acquire_minor(major, MinorId::new(0), "conflict").is_err());
        drop(owner);
        let partial = acquire_minor(major, MinorId::MAX, "partial").unwrap();
        let other = allocate_major("other").unwrap();
        assert_ne!(other.get(), major);
        drop((partial, other));
        let _whole = acquire_major(major, "whole").unwrap();
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
