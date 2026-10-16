// SPDX-License-Identifier: MPL-2.0

//! Worker execution and event waiting for vhost backends.

#![short_vis_path::add(vhost)]

use super::device::VhostSharedState;
use crate::{
    events::IoEvents,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

/// Whether a backend can continue processing without another notification.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in vhost) enum VhostWorkStatus {
    /// No immediately actionable work remains; wait for a notification.
    Idle,
    /// More work can be attempted without a notification.
    Pending,
}

pub(super) fn run<const NUM_QUEUES: usize>(
    shared: &VhostSharedState<NUM_QUEUES>,
    work_fn: &mut impl FnMut(&VhostSharedState<NUM_QUEUES>) -> VhostWorkStatus,
) {
    loop {
        // A new poller drops the previous kick registrations. Internal wake
        // remains independent of the optional guest kick eventfds.
        let mut poller = Poller::new(None);
        shared.worker_pollee().poll_with(
            IoEvents::IN,
            Some(poller.as_handle_mut()),
            IoEvents::empty,
        );
        let runtime = shared.runtime().lock();
        if shared.stop_worker_requested() {
            return;
        }
        if runtime.is_running() {
            for index in 0..NUM_QUEUES {
                if let Some(kick) = runtime.kick_event(index) {
                    kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
                    kick.consume();
                }
            }
        }
        drop(runtime);
        // Configuration changes wake this poller even if they replace a kick
        // after registration. The callback rechecks state under its own lock.
        if work_fn(shared) == VhostWorkStatus::Pending {
            Thread::yield_now();
        } else {
            // This is a kernel thread (no POSIX signals), with no timeout.
            poller.wait().unwrap();
        }
    }
}
