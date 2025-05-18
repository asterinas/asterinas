// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support

use crate::{boot::smp::PerApRawInfo, mm::Paddr};

pub(crate) fn count_processors() -> Option<u32> {
    Some(1)
}

pub(crate) fn bringup_all_aps(_info_ptr: *const PerApRawInfo, _pr_ptr: Paddr, _num_cpus: u32) {
    unimplemented!()
}
