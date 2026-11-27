// SPDX-License-Identifier: MPL-2.0

//! Deferred BIO completion in softirq context.

use core::cell::RefCell;

use aster_softirq::{SoftIrqLine, softirq_id::BLOCK_SOFTIRQ_ID};
use intrusive_collections::LinkedList;
use ostd::{cpu_local, irq};

use crate::bio::{BioCompletionAdapter, SubmittedBio};

cpu_local! {
    /// Stores BIO completions deferred on the current CPU.
    static BIO_COMPLETION_QUEUE: RefCell<LinkedList<BioCompletionAdapter>> =
        RefCell::new(LinkedList::new(BioCompletionAdapter::NEW));
}

/// Initializes the block softirq handler.
pub(super) fn init() {
    SoftIrqLine::get(BLOCK_SOFTIRQ_ID).enable(handle_block_softirq);
}

/// Enqueues a BIO completion and raises the block softirq if needed.
pub(super) fn enqueue(bio: SubmittedBio) {
    let irq_guard = irq::disable_local();
    let queue = BIO_COMPLETION_QUEUE.get_with(&irq_guard);
    let mut queue = queue.borrow_mut();
    let should_raise = queue.is_empty();
    queue.push_back(bio.into_inner());
    drop(queue);

    if should_raise {
        SoftIrqLine::get(BLOCK_SOFTIRQ_ID).raise();
    }
}

/// Drains deferred BIO completions for the current CPU.
fn handle_block_softirq() {
    let mut completion_queue = {
        let irq_guard = irq::disable_local();
        let queue = BIO_COMPLETION_QUEUE.get_with(&irq_guard);
        queue.borrow_mut().take()
    };

    while let Some(completion) = completion_queue.pop_front() {
        completion.complete_now();
    }
}
