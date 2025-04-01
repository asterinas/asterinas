// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support

pub(crate) fn count_processors() -> Option<u32> {
    Some(1)
}

pub(crate) fn bringup_all_aps() {
    // TODO
}
