// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) struct NvmeStats {
    completed: AtomicU64,
    submitted: AtomicU64,
}

impl NvmeStats {
    pub(super) fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            submitted: AtomicU64::new(0),
        }
    }

    #[expect(dead_code)]
    pub(super) fn get_stats(&self) -> (u64, u64) {
        (
            self.submitted.load(Ordering::Relaxed),
            self.completed.load(Ordering::Relaxed),
        )
    }

    #[expect(dead_code)]
    pub(super) fn reset_stats(&self) {
        self.submitted.store(0, Ordering::Relaxed);
        self.completed.store(0, Ordering::Relaxed);
    }

    pub(super) fn increment_submitted(&self) {
        self.submitted.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn increment_completed(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
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
