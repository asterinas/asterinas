// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use aster_softirq::{BottomHalfDisabled, Taskless};
use ostd::sync::SpinLock;
use spin::Once;

use crate::net::socket::vsock::transport::{conn_id::ConnId, space::vsock_space};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Returns a unique identifier for a timer event.
///
/// This value wraps around on `u64` overflow, so uniqueness is only guaranteed until the counter
/// overflows. In practice, that should take long enough to be acceptable here.
pub(super) fn next_timer_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

pub(super) struct TimerEvent {
    pub(super) conn_id: ConnId,
    pub(super) generation: u64,
}

static PENDING_EVENTS: SpinLock<Vec<TimerEvent>, BottomHalfDisabled> = SpinLock::new(Vec::new());

static TASKLESS: Once<Arc<Taskless>> = Once::new();

pub(super) fn push_timer_event(conn_id: ConnId, generation: u64) {
    let event = TimerEvent {
        conn_id,
        generation,
    };
    PENDING_EVENTS.lock().push(event);

    TASKLESS.get().unwrap().schedule();
}

fn process_pending_timer_events() {
    let events = {
        let mut pending = PENDING_EVENTS.lock();
        core::mem::take(&mut *pending)
    };

    let vsock_space = vsock_space().unwrap();
    vsock_space.process_timer_events(events);
}

pub(super) fn init() {
    TASKLESS.call_once(|| Taskless::new(process_pending_timer_events));
}
