// SPDX-License-Identifier: MPL-2.0

use alloc::collections::btree_set::BTreeSet;

use ostd::{sync::RwLock, Error};

const BLOCK_MAJOR_MAX: u32 = 512;
const BLOCK_LAST_DYNAMIC_MAJOR: u32 = 254;

static BLOCK_MAJORS: RwLock<BTreeSet<u32>> = RwLock::new(BTreeSet::new());

/// Registers a block device major number.
///
/// This function registers a major device number for block devices. If the
/// requested major number is 0, a dynamic major number will be allocated.
pub(crate) fn register_device_ids(major: u32) -> Result<u32, Error> {
    if major >= BLOCK_MAJOR_MAX {
        return Err(Error::InvalidArgs);
    }

    let mut majors = BLOCK_MAJORS.write();
    if major == 0 {
        for id in (1..BLOCK_LAST_DYNAMIC_MAJOR + 1).rev() {
            if majors.insert(id) {
                return Ok(id);
            }
        }
        return Err(Error::NotEnoughResources);
    }

    if majors.insert(major) {
        return Ok(major);
    }

    Err(Error::NotEnoughResources)
}

/// Unregisters a block device major number.
pub(crate) fn unregister_device_ids(major: u32) {
    let _ = BLOCK_MAJORS.write().remove(&major);
}

#[cfg(ktest)]
mod tests {
    use ostd::{prelude::*, Error};

    use super::{
        register_device_ids, unregister_device_ids, BLOCK_LAST_DYNAMIC_MAJOR, BLOCK_MAJOR_MAX,
    };

    #[ktest]
    fn test_register_valid_major() {
        let major = 10;
        let result = register_device_ids(major);
        assert_eq!(result, Ok(major));

        unregister_device_ids(major);
    }

    #[ktest]
    fn test_register_invalid_major() {
        let major = BLOCK_MAJOR_MAX;
        let result = register_device_ids(major);
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    #[ktest]
    fn test_register_duplicate_major() {
        let major = 20;
        let result1 = register_device_ids(major);
        assert_eq!(result1, Ok(major));

        let result2 = register_device_ids(major);
        assert_eq!(result2, Err(Error::NotEnoughResources));

        unregister_device_ids(major);
    }

    #[ktest]
    fn test_register_dynamic_major() {
        let result = register_device_ids(0);
        assert!(result.is_ok());
        let allocated_major = result.unwrap();
        assert!(allocated_major > 0);
        assert!(allocated_major <= BLOCK_LAST_DYNAMIC_MAJOR);

        unregister_device_ids(allocated_major);
    }

    #[ktest]
    fn test_unregister_registered_major() {
        let major = 30;
        assert_eq!(register_device_ids(major), Ok(major));

        unregister_device_ids(major);

        assert_eq!(register_device_ids(major), Ok(major));

        unregister_device_ids(major);
    }

    #[ktest]
    fn test_unregister_unregistered_major() {
        let major = 40;
        unregister_device_ids(major);
    }

    #[ktest]
    fn test_dynamic_allocation_order() {
        let result1 = register_device_ids(0);
        let result2 = register_device_ids(0);
        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let major1 = result1.unwrap();
        let major2 = result2.unwrap();

        assert!(major2 < major1);

        unregister_device_ids(major1);
        unregister_device_ids(major2);
    }
}
