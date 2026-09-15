// SPDX-License-Identifier: MPL-2.0

//! A pausable, owner-VMAR-bound worker shared by vhost device backends.
//!
//! Provides the execution and event-waiting roles of Linux's `vhost_worker`
//! and `vhost_poll`, using Asterinas threads and pollers.
//! Reference: <https://github.com/torvalds/linux/blob/v6.18/drivers/vhost/vhost.h>.

#![short_vis_path::add(vhost)]

use core::sync::atomic::{AtomicBool, Ordering};

use super::device::VhostDeviceData;
use crate::{
    events::{IoEvents, KernelEventFile},
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::{Thread, kernel_thread::ThreadOptions},
    vm::vmar::Vmar,
};

/// Whether a backend can continue processing without another notification.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in vhost) enum VhostWorkStatus {
    Idle,
    Pending,
}

/// Device-specific work executed by a common worker.
pub(in vhost) trait VhostWork<const NUM_QUEUES: usize>: Send + 'static {
    /// Processes enabled queues while the device data mutex is held.
    ///
    /// The worker registers and consumes kicks before calling this method.
    /// The backend chooses its batch budget and notification suppression policy.
    /// Before returning `Idle`, it must re-enable kicks and check for requests
    /// that raced with re-enabling; `Pending` continues without waiting.
    fn process(&mut self, device: &mut VhostDeviceData<NUM_QUEUES>) -> Result<VhostWorkStatus>;

    /// Runs after unlocking the device, including when its queues are paused.
    fn after_process(&mut self, _status: VhostWorkStatus) -> Result<()> {
        Ok(())
    }

    /// Handles the terminal result after releasing the data mutex.
    fn on_exit(self, result: Result<()>);
}

/// Thread and exit state owned under the common device's worker mutex.
///
/// The running thread captures device data and never acquires the worker mutex.
#[derive(Default)]
pub(super) struct VhostWorker {
    thread: Option<Arc<Thread>>,
    exiting: Arc<AtomicBool>,
}

impl VhostWorker {
    /// Starts work in the owner's address space, including after worker failure.
    pub(super) fn start<const NUM_QUEUES: usize>(
        &mut self,
        vmar: Arc<Vmar>,
        device: Arc<Mutex<VhostDeviceData<NUM_QUEUES>>>,
        wake: Arc<KernelEventFile>,
        mut work: impl VhostWork<NUM_QUEUES>,
    ) {
        assert!(self.thread.is_none());
        self.exiting.store(false, Ordering::Release);
        let exiting = self.exiting.clone();
        self.thread = Some(
            ThreadOptions::new(move || {
                let result = Self::run(&device, &wake, &exiting, &mut work);
                work.on_exit(result);
            })
            .vmar(vmar)
            .spawn(),
        );
    }

    /// Wakes and joins the worker after the device has disabled its queues.
    pub(super) fn stop(&mut self, wake: &KernelEventFile) {
        self.exiting.store(true, Ordering::Release);
        wake.signal();
        if let Some(thread) = self.thread.take() {
            thread.join();
        }
    }

    fn run<const NUM_QUEUES: usize>(
        device: &Mutex<VhostDeviceData<NUM_QUEUES>>,
        wake: &KernelEventFile,
        exiting: &AtomicBool,
        work: &mut impl VhostWork<NUM_QUEUES>,
    ) -> Result<()> {
        loop {
            // A new poller drops the previous kick registrations. Internal wake
            // remains independent of the optional guest kick eventfds.
            let mut poller = Poller::new(None);
            wake.poll(IoEvents::IN, Some(poller.as_handle_mut()));
            wake.consume();
            let mut device = device.lock();
            if exiting.load(Ordering::Acquire) {
                return Ok(());
            }
            let status = if device.is_running() {
                for index in 0..NUM_QUEUES {
                    if let Some(kick) = device.kick_event(index) {
                        kick.poll(IoEvents::IN, Some(poller.as_handle_mut()));
                        kick.consume();
                    }
                }
                work.process(&mut device)?
            } else {
                VhostWorkStatus::Idle
            };
            drop(device);
            work.after_process(status)?;
            if status == VhostWorkStatus::Pending {
                Thread::yield_now();
            } else {
                poller.wait()?;
            }
        }
    }
}
