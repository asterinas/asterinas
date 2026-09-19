// SPDX-License-Identifier: MPL-2.0

//! A pausable, owner-VMAR-bound worker shared by vhost device backends.
//!
//! Provides the execution and event-waiting roles of Linux's
//! [`vhost_worker`](https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.h#L39)
//! and [`vhost_poll`](https://elixir.bootlin.com/linux/v6.18/source/drivers/vhost/vhost.h#L55),
//! using Asterinas threads and pollers.

#![short_vis_path::add(vhost)]

use super::device::VhostSharedData;
use crate::{
    events::IoEvents,
    process::signal::{Pollable, Poller},
    thread::Thread,
};

/// Whether a backend can continue processing without another notification.
///
/// A backend may limit each batch to bound runtime-lock hold time and keep
/// control ioctls responsive. `Pending` releases that lock and yields execution
/// before continuing without another kick. It does not guarantee fairness among
/// queues; scheduling work between queues remains the backend's responsibility.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in vhost) enum VhostWorkStatus {
    /// No immediately actionable work remains; wait for a notification.
    Idle,
    /// More work can be attempted without a notification.
    Pending,
}

/// Device-specific work executed by a common worker.
pub(in vhost) trait VhostWork: Send + 'static {
    /// Processes work using the shared state of the session that started it.
    ///
    /// The worker registers and consumes kicks before this call, then releases
    /// the runtime mutex. This method is also called while queues are paused.
    /// It must lock runtime data and check that queues are still enabled before
    /// accessing them, since an ioctl may have changed their state meanwhile.
    ///
    /// The backend chooses its batch budget and notification suppression policy.
    /// Before returning `Idle`, it must re-enable kicks and check for requests
    /// that raced with re-enabling; `Pending` continues without waiting.
    ///
    /// Queue errors must be reported and handled here, for example by signaling
    /// the affected error eventfd and ending the batch. They do not terminate
    /// the worker. Endpoint failure and cleanup policy belong to the backend.
    fn process(&mut self) -> VhostWorkStatus;
}

pub(super) fn run<const NUM_QUEUES: usize>(
    shared: &VhostSharedData<NUM_QUEUES>,
    work: &mut impl VhostWork,
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
        let runtime = shared.lock();
        if shared.stop_requested() {
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
        if work.process() == VhostWorkStatus::Pending {
            Thread::yield_now();
        } else {
            // This is a kernel thread (no POSIX signals), with no timeout.
            poller.wait().unwrap();
        }
    }
}
