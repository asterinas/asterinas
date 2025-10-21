// SPDX-License-Identifier: MPL-2.0// SPDX-License-Identifier: MPL-2.0

use alloc::collections::{
    btree_map::{BTreeMap, Entry},
    linked_list::LinkedList,
};
use core::ops::Range;

use ostd::{sync::RwLock, Error};

const CHAR_MAJOR_MAX: u32 = 512;
const CHAR_MINORS_MAX: u32 = 1 << super::id::MINOR_BITS;

const CHAR_FIRST_DYNAMIC_MAJOR_START: u32 = 234;
const CHAR_FIRST_DYNAMIC_MAJOR_END: u32 = 254;
const CHAR_SECOND_DYNAMIC_MAJOR_START: u32 = 384;
const CHAR_SECOND_DYNAMIC_MAJOR_END: u32 = 511;

static CHAR_MAJORS: RwLock<BTreeMap<u32, LinkedList<Range<u32>>>> = RwLock::new(BTreeMap::new());

/// Registers character device major and minor numbers.
///
/// This function registers a major device number and a range of minor device
/// numbers for character devices. If the requested major number is 0, a dynamic
/// major number will be allocated.
pub(crate) fn register_device_ids(major: u32, minors: &Range<u32>) -> Result<u32, Error> {
    if major >= CHAR_MAJOR_MAX || minors.end > CHAR_MINORS_MAX {
        return Err(Error::InvalidArgs);
    }

    let mut majors = CHAR_MAJORS.write();
    if major == 0 {
        for id in (CHAR_FIRST_DYNAMIC_MAJOR_START..CHAR_FIRST_DYNAMIC_MAJOR_END + 1).rev() {
            if let Entry::Vacant(e) = majors.entry(id) {
                let mut list = LinkedList::new();
                list.push_back(minors.clone());
                e.insert(list);
                return Ok(id);
            }
        }
        for id in (CHAR_SECOND_DYNAMIC_MAJOR_START..CHAR_SECOND_DYNAMIC_MAJOR_END + 1).rev() {
            if let Entry::Vacant(e) = majors.entry(id) {
                let mut list = LinkedList::new();
                list.push_back(minors.clone());
                e.insert(list);
                return Ok(id);
            }
        }
        return Err(Error::NotEnoughResources);
    }

    if let Entry::Vacant(e) = majors.entry(major) {
        let mut list = LinkedList::new();
        list.push_back(minors.clone());
        e.insert(list);
        return Ok(major);
    }

    let mut cursor = majors.get_mut(&major).unwrap().cursor_front_mut();
    while let Some(current) = cursor.current() {
        if minors.end <= current.start {
            cursor.insert_before(minors.clone());
            return Ok(major);
        }
        if minors.start >= current.end {
            cursor.move_next();
            continue;
        }
        return Err(Error::NotEnoughResources);
    }
    cursor.insert_before(minors.clone());

    Ok(major)
}

/// Unregisters character device major and minor numbers.
pub(crate) fn unregister_device_ids(major: u32, minors: &Range<u32>) {
    let mut majors = CHAR_MAJORS.write();
    if !majors.contains_key(&major) {
        return;
    }

    let list = majors.get_mut(&major).unwrap();
    let mut cursor = list.cursor_front_mut();
    while let Some(current) = cursor.current() {
        if minors == current {
            cursor.remove_current();
            break;
        }
        cursor.move_next();
    }

    if list.is_empty() {
        let _ = majors.remove(&major);
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::{prelude::*, Error};

    use super::{
        register_device_ids, unregister_device_ids, CHAR_FIRST_DYNAMIC_MAJOR_END,
        CHAR_FIRST_DYNAMIC_MAJOR_START, CHAR_MAJOR_MAX, CHAR_MINORS_MAX,
        CHAR_SECOND_DYNAMIC_MAJOR_END, CHAR_SECOND_DYNAMIC_MAJOR_START,
    };

    #[ktest]
    fn test_register_valid_major() {
        let major = 10;
        let minors = 0..10;
        let result = register_device_ids(major, &minors);
        assert_eq!(result, Ok(major));

        unregister_device_ids(major, &minors);
    }

    #[ktest]
    fn test_register_invalid_major() {
        let major = CHAR_MAJOR_MAX;
        let minors = 0..10;
        let result = register_device_ids(major, &minors);
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    #[ktest]
    fn test_register_invalid_minors() {
        let major = 10;
        let minors = 0..(CHAR_MINORS_MAX + 1);
        let result = register_device_ids(major, &minors);
        assert_eq!(result, Err(Error::InvalidArgs));
    }

    #[ktest]
    fn test_register_duplicate_major() {
        let major = 20;
        let minors1 = 0..10;
        let minors2 = 10..20;

        let result1 = register_device_ids(major, &minors1);
        assert_eq!(result1, Ok(major));

        let result2 = register_device_ids(major, &minors2);
        assert_eq!(result2, Ok(major));

        unregister_device_ids(major, &minors1);
        unregister_device_ids(major, &minors2);
    }

    #[ktest]
    fn test_register_overlapping_minors() {
        let major = 30;
        let minors1 = 0..10;
        let minors2 = 5..15;

        let result1 = register_device_ids(major, &minors1);
        assert_eq!(result1, Ok(major));

        let result2 = register_device_ids(major, &minors2);
        assert_eq!(result2, Err(Error::NotEnoughResources));

        unregister_device_ids(major, &minors1);
    }

    #[ktest]
    fn test_register_non_overlapping_minors() {
        let major = 40;
        let minors1 = 0..10;
        let minors2 = 15..25;

        let result1 = register_device_ids(major, &minors1);
        assert_eq!(result1, Ok(major));

        let result2 = register_device_ids(major, &minors2);
        assert_eq!(result2, Ok(major));

        unregister_device_ids(major, &minors1);
        unregister_device_ids(major, &minors2);
    }

    #[ktest]
    fn test_register_dynamic_major_first_range() {
        for id in CHAR_FIRST_DYNAMIC_MAJOR_START..=CHAR_FIRST_DYNAMIC_MAJOR_END {
            let minors = 0..10;
            unregister_device_ids(id, &minors);
        }

        let minors = 0..10;
        let result = register_device_ids(0, &minors);
        assert!(result.is_ok());
        let allocated_major = result.unwrap();
        assert!(allocated_major >= CHAR_FIRST_DYNAMIC_MAJOR_START);
        assert!(allocated_major <= CHAR_FIRST_DYNAMIC_MAJOR_END);

        unregister_device_ids(allocated_major, &minors);
    }

    #[ktest]
    fn test_register_dynamic_major_second_range() {
        for id in CHAR_FIRST_DYNAMIC_MAJOR_START..=CHAR_FIRST_DYNAMIC_MAJOR_END {
            let minors = 0..10;
            let _ = register_device_ids(id, &minors);
        }

        let minors = 0..10;
        let result = register_device_ids(0, &minors);
        assert!(result.is_ok());
        let allocated_major = result.unwrap();
        assert!(allocated_major >= CHAR_SECOND_DYNAMIC_MAJOR_START);
        assert!(allocated_major <= CHAR_SECOND_DYNAMIC_MAJOR_END);

        for id in CHAR_FIRST_DYNAMIC_MAJOR_START..=CHAR_FIRST_DYNAMIC_MAJOR_END {
            let minors = 0..10;
            unregister_device_ids(id, &minors);
        }
        unregister_device_ids(allocated_major, &minors);
    }

    #[ktest]
    fn test_unregister_registered_major() {
        let major = 50;
        let minors = 0..10;
        assert_eq!(register_device_ids(major, &minors), Ok(major));

        unregister_device_ids(major, &minors);

        assert_eq!(register_device_ids(major, &minors), Ok(major));

        unregister_device_ids(major, &minors);
    }

    #[ktest]
    fn test_unregister_unregistered_major() {
        let major = 60;
        let minors = 0..10;
        unregister_device_ids(major, &minors);
    }
}
