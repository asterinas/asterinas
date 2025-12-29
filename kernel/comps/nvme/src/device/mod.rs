// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::{self, Debug},
    sync::atomic::{AtomicU64, Ordering},
};

pub mod block_device;

pub(crate) const MAX_NS_NUM: usize = 1024;

#[derive(Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeDeviceError {
    MsixAllocationFailed,
    NoNamespace,
    QueuesAmountDoNotMatch,
}

pub(crate) struct NvmeStats {
    completed: AtomicU64,
    submitted: AtomicU64,
}

impl NvmeStats {
    pub(crate) fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            submitted: AtomicU64::new(0),
        }
    }

    #[expect(dead_code)]
    pub(crate) fn get_stats(&self) -> (u64, u64) {
        (
            self.submitted.load(Ordering::Relaxed),
            self.completed.load(Ordering::Relaxed),
        )
    }

    #[expect(dead_code)]
    pub(crate) fn reset_stats(&self) {
        self.submitted.store(0, Ordering::Relaxed);
        self.completed.store(0, Ordering::Relaxed);
    }

    pub(crate) fn increment_submitted(&self) {
        self.submitted.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn increment_completed(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
#[expect(dead_code)]
pub(crate) struct NvmeNamespace {
    pub(crate) id: u32,
    pub(crate) free_blocks: u64,
    pub(crate) used_blocks: u64,
    pub(crate) block_size: u64,
}

impl fmt::Display for NvmeStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "submitted {} completed {}",
            self.submitted.load(Ordering::Relaxed),
            self.completed.load(Ordering::Relaxed)
        )
    }
}
