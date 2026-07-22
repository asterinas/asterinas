// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::sync::WaitQueue;

use super::vm::Vm;
use crate::{
    events::{IoEvents, Observer},
    prelude::*,
    process::signal::{PollAdaptor, Pollable},
    syscall::eventfd::EventFile,
    thread::work_queue::{WorkPriority, submit_work_item, work_item::WorkItem},
};

pub(super) struct IrqFdBinding {
    eventfd: Arc<EventFile>,
    gsi: u32,
    // vm: Weak<Vm>,
    poll_adaptor: Mutex<Option<PollAdaptor<IrqFdObserver>>>,
    work_item: Arc<WorkItem>,
    /// Whether this binding is still active.
    active: AtomicBool,
    /// Whether a work item is currently scheduled for this binding.
    scheduled: AtomicBool,
    work_done: WaitQueue,
}

impl IrqFdBinding {
    pub(super) fn new(eventfd: Arc<EventFile>, gsi: u32, _vm: Weak<Vm>) -> Arc<Self> {
        Arc::new_cyclic(|binding| {
            let observer = IrqFdObserver {
                binding: binding.clone(),
            };
            let work_binding = binding.clone();
            Self {
                eventfd,
                gsi,
                // vm,
                poll_adaptor: Mutex::new(Some(PollAdaptor::with_observer(observer))),
                work_item: WorkItem::new(Box::new(move || {
                    if let Some(binding) = work_binding.upgrade() {
                        binding.run_work();
                    }
                })),
                active: AtomicBool::new(true),
                scheduled: AtomicBool::new(false),
                work_done: WaitQueue::new(),
            }
        })
    }

    pub(super) fn start(&self) {
        let ready = {
            let mut poll_adaptor = self.poll_adaptor.lock();
            let poll_adaptor = poll_adaptor.as_mut().unwrap();
            self.eventfd
                .poll(IoEvents::IN, Some(poll_adaptor.as_handle_mut()))
        };
        if ready.contains(IoEvents::IN) {
            self.schedule();
        }
    }

    pub(super) fn matches(&self, eventfd: &Arc<EventFile>, gsi: u32) -> bool {
        self.gsi == gsi && Arc::ptr_eq(&self.eventfd, eventfd)
    }

    pub(super) fn uses_eventfd(&self, eventfd: &Arc<EventFile>) -> bool {
        Arc::ptr_eq(&self.eventfd, eventfd)
    }

    pub(super) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
        self.poll_adaptor.lock().take();
        self.work_done
            .wait_until(|| (!self.scheduled.load(Ordering::Acquire)).then_some(()));
    }

    fn schedule(&self) {
        if !self.active.load(Ordering::Acquire)
            || self
                .scheduled
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }

        if !submit_work_item(self.work_item.clone(), WorkPriority::High) {
            // If we failed to submit the work item
            self.scheduled.store(false, Ordering::Release);
            self.work_done.wake_all();
        }
    }

    fn run_work(&self) {
        #[expect(
            clippy::missing_spin_loop,
            reason = "GSI injection will be implemented here"
        )]
        while self.active.load(Ordering::Acquire) && self.eventfd.consume().is_some() {
            // TODO: Inject a GSI interrupt into `_vm` for `self.gsi` here.
        }

        // All signal is consumed or the binding is no longer active.
        self.scheduled.store(false, Ordering::Release);
        self.work_done.wake_all();

        // A signal between the last consume and clearing `scheduled` may fail to enqueue the
        // work item. Recheck the eventfd to catch that case.
        if self.active.load(Ordering::Acquire)
            && self.eventfd.poll(IoEvents::IN, None).contains(IoEvents::IN)
        {
            self.schedule();
        }
    }
}

struct IrqFdObserver {
    binding: Weak<IrqFdBinding>,
}

impl Observer<IoEvents> for IrqFdObserver {
    fn on_events(&self, events: &IoEvents) {
        if events.contains(IoEvents::IN)
            && let Some(binding) = self.binding.upgrade()
        {
            binding.schedule();
        }
    }
}
