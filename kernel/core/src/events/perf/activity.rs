// SPDX-License-Identifier: MPL-2.0

//! A sampling hint whose lifetime includes deferred removal of closed events.

use core::sync::atomic::{AtomicUsize, Ordering};

static EVENT_RECORDS: AtomicUsize = AtomicUsize::new(0);

/// Keeps timer sampling enabled until the owning event record is removed.
#[derive(Debug)]
pub(super) struct EventActivity;

impl EventActivity {
    pub(super) fn new() -> Self {
        EVENT_RECORDS.fetch_add(1, Ordering::Release);
        Self
    }
}

impl Drop for EventActivity {
    fn drop(&mut self) {
        EVENT_RECORDS.fetch_sub(1, Ordering::Release);
    }
}

pub(super) fn has_events() -> bool {
    EVENT_RECORDS.load(Ordering::Acquire) != 0
}
