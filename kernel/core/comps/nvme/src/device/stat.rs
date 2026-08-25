// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub(super) struct NvmeStats {
    completed: AtomicU64,
    submitted: AtomicU64,
    in_flight: AtomicU64,
    max_in_flight: AtomicU64,
}

impl NvmeStats {
    pub(super) fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            submitted: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            max_in_flight: AtomicU64::new(0),
        }
    }

    #[cfg(ktest)]
    pub(super) fn reset_stats(&self) {
        self.submitted.store(0, Ordering::Relaxed);
        self.completed.store(0, Ordering::Relaxed);
        self.in_flight.store(0, Ordering::Relaxed);
        self.max_in_flight.store(0, Ordering::Relaxed);
    }

    pub(super) fn increment_submitted(&self) {
        self.submitted.fetch_add(1, Ordering::Relaxed);
        let cur = self.in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        let prev_max = self.max_in_flight.fetch_max(cur, Ordering::Relaxed);
        if cur > prev_max && cur > 1 {
            ostd::info!(
                "I/O queue peak in-flight commands increased to {} (submitted {}, completed {})",
                cur,
                self.submitted.load(Ordering::Relaxed),
                self.completed.load(Ordering::Relaxed)
            );
        }
    }

    pub(super) fn increment_completed(&self) {
        self.completed.fetch_add(1, Ordering::Relaxed);
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }

    #[cfg(ktest)]
    pub(super) fn max_in_flight(&self) -> u64 {
        self.max_in_flight.load(Ordering::Relaxed)
    }
}

impl fmt::Display for NvmeStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "submitted {} completed {} in_flight {} max_in_flight {}",
            self.submitted.load(Ordering::Relaxed),
            self.completed.load(Ordering::Relaxed),
            self.in_flight.load(Ordering::Relaxed),
            self.max_in_flight.load(Ordering::Relaxed)
        )
    }
}
