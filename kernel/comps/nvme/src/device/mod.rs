// SPDX-License-Identifier: MPL-2.0

use core::fmt::{self, Debug};

pub mod block_device;

pub(crate) const MAX_NS_NUM: usize = 1024;

#[derive(Debug)]
pub(crate) enum NVMeDeviceError {
    QueuesAmountDoNotMatch,
}

#[derive(Debug)]
pub(crate) struct NVMeStats {
    completed: u64,
    submitted: u64,
}

impl NVMeStats {
    pub(crate) fn get_stats(&self) -> (u64, u64) {
        (self.submitted, self.completed)
    }
    pub(crate) fn reset_stats(&mut self) {
        self.submitted = 0;
        self.completed = 0;
    }
}

#[derive(Debug)]
pub(crate) struct NVMeNamespace {
    pub(crate) id: u32,
    pub(crate) free_blocks: u64,
    pub(crate) used_blocks: u64,
    pub(crate) block_size: u64,
}

impl fmt::Display for NVMeStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "submitted {} completed {}",
            self.submitted, self.completed
        )
    }
}
