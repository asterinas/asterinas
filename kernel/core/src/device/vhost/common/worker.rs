// SPDX-License-Identifier: MPL-2.0

//! A pausable, owner-VMAR-bound worker shared by vhost device backends.
//!
//! Provides the execution and event-waiting roles of Linux's `vhost_worker`
//! and `vhost_poll`, using Asterinas threads and pollers.
//! Reference: <https://github.com/torvalds/linux/blob/v6.18/drivers/vhost/vhost.h>.

#![short_vis_path::add(vhost)]

use core::sync::atomic::{AtomicBool, Ordering};

use super::device::{VhostDevice, ioctl_defs::SetOwner};
use crate::{
    events::{EventFile, EventFileFlags, IoEvents, KernelEventFile},
    prelude::*,
    process::signal::{Pollable, Poller},
    thread::{Thread, kernel_thread::ThreadOptions},
    util::ioctl::RawIoctl,
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
    /// Processes enabled queues while the device mutex is held.
    ///
    /// The worker registers and consumes kicks before calling this method.
    /// The backend chooses its batch budget and notification suppression policy.
    /// Before returning `Idle`, it must re-enable kicks and check for requests
    /// that raced with re-enabling; `Pending` continues without waiting.
    fn process(&mut self, device: &mut VhostDevice<NUM_QUEUES>) -> Result<VhostWorkStatus>;

    /// Runs after unlocking the device, including when its queues are paused.
    fn after_process(&mut self, _status: VhostWorkStatus) -> Result<()> {
        Ok(())
    }

    /// Handles the terminal result after releasing the device mutex.
    fn on_exit(self, result: Result<()>);
}

/// Owns a worker's control path; the backend serializes access to this object.
///
/// The device mutex protects configuration and each processing batch. The worker
/// never acquires the backend's control lock, so stopping can join it safely.
/// Session close must stop the worker even if its work retains backend references.
pub(in vhost) struct VhostWorker<const NUM_QUEUES: usize> {
    device: Arc<Mutex<VhostDevice<NUM_QUEUES>>>,
    thread: Option<Arc<Thread>>,
    wake: Arc<KernelEventFile>,
    exiting: Arc<AtomicBool>,
}

impl<const NUM_QUEUES: usize> VhostWorker<NUM_QUEUES> {
    pub(in vhost) fn new(device: Arc<Mutex<VhostDevice<NUM_QUEUES>>>) -> Self {
        Self {
            device,
            thread: None,
            wake: KernelEventFile::from_file(&EventFile::new(0, EventFileFlags::empty())).unwrap(),
            exiting: Arc::new(AtomicBool::new(false)),
        }
    }

    /// An internal notification for configuration changes and backend work.
    pub(in vhost) fn wake_event(&self) -> &Arc<KernelEventFile> {
        &self.wake
    }

    /// Handles common ioctls, creates the owner worker, and wakes it after updates.
    pub(in vhost) fn handle_ioctl(
        &mut self,
        raw: RawIoctl,
        work: impl VhostWork<NUM_QUEUES>,
    ) -> Result<i32> {
        let result = self.device.lock().handle_ioctl(raw);
        if result.is_ok() && SetOwner::try_from_raw(raw).is_some() {
            let vmar = self.device.lock().owner_vmar().unwrap().clone();
            self.start(vmar, work);
        }
        self.wake.signal();
        result
    }

    /// Starts work in the owner's address space, including after worker failure.
    pub(in vhost) fn start(&mut self, vmar: Arc<Vmar>, mut work: impl VhostWork<NUM_QUEUES>) {
        assert!(self.thread.is_none());
        self.exiting.store(false, Ordering::Release);
        let device = self.device.clone();
        let wake = self.wake.clone();
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

    pub(in vhost) fn activate(&self) -> Result<()> {
        let result = self.device.lock().activate();
        self.wake.signal();
        result
    }

    pub(in vhost) fn deactivate(&self) {
        self.device.lock().deactivate();
        self.wake.signal();
    }

    /// Disables queues and joins the worker without holding the device mutex.
    pub(in vhost) fn stop(&mut self) {
        self.device.lock().deactivate();
        self.exiting.store(true, Ordering::Release);
        self.wake.signal();
        if let Some(thread) = self.thread.take() {
            thread.join();
        }
    }

    fn run(
        device: &Mutex<VhostDevice<NUM_QUEUES>>,
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

impl<const NUM_QUEUES: usize> Drop for VhostWorker<NUM_QUEUES> {
    fn drop(&mut self) {
        self.stop();
    }
}
