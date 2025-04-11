// SPDX-License-Identifier: MPL-2.0

use core::fmt::{self, Debug};

pub mod block_device;

pub const MAX_NS_NUM: usize = 1024;

#[derive(Debug)]
pub enum NVMeDeviceError {
    QueuesAmountDoNotMatch,
}

#[derive(Debug)]
pub struct NVMeStats {
    completed: u64,
    submitted: u64,
}

impl NVMeStats {
    pub fn get_stats(&self) -> (u64, u64) {
        (self.submitted, self.completed)
    }
    pub fn reset_stats(&mut self) {
        self.submitted = 0;
        self.completed = 0;
    }
}

#[derive(Debug)]
pub struct NVMeNamespace {
    pub id: u32,
    pub free_blocks: u64,
    pub used_blocks: u64,
    pub block_size: u64,
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
