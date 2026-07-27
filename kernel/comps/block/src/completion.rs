// SPDX-License-Identifier: MPL-2.0

//! Provides logic to defer BIO completion to the softirq context.

use core::{cell::RefCell, mem};

use aster_softirq::{SoftIrqLine, softirq_id::BLOCK_SOFTIRQ_ID};
use component::ComponentInitError;
use ostd::{cpu_local, irq};

use crate::{
    bio::{BioStatus, SubmittedBio},
    prelude::*,
};

/// A pending BIO completion deferred to the block softirq.
struct BioCompletion {
    /// The submitted BIO whose ownership is transferred to the completion path.
    bio: SubmittedBio,
    /// The final status reported by the block driver.
    status: BioStatus,
}

cpu_local! {
    /// Stores BIO completions deferred on the current CPU.
    static BIO_COMPLETION_QUEUE: RefCell<VecDeque<BioCompletion>> =
        RefCell::new(VecDeque::new());
}

/// Initializes the block softirq handler.
pub(super) fn init() -> Result<(), ComponentInitError> {
    SoftIrqLine::get(BLOCK_SOFTIRQ_ID).enable(handle_block_softirq);
    Ok(())
}

/// Enqueues a BIO completion and raises the block softirq if needed.
///
/// The queue is CPU-local, so local IRQs are disabled while the queue is
/// accessed. A softirq is raised only when the queue transitions from empty to
/// non-empty, which avoids redundant raises while preserving progress.
pub(super) fn enqueue(bio: SubmittedBio, status: BioStatus) {
    let irq_guard = irq::disable_local();
    let queue = BIO_COMPLETION_QUEUE.get_with(&irq_guard);
    let mut queue = queue.borrow_mut();
    let should_raise = queue.is_empty();
    queue.push_back(BioCompletion { bio, status });
    drop(queue);

    if should_raise {
        SoftIrqLine::get(BLOCK_SOFTIRQ_ID).raise();
    }
}

/// Drains deferred BIO completions for the current CPU.
fn handle_block_softirq() {
    let mut completion_queue = VecDeque::new();

    {
        let irq_guard = irq::disable_local();
        let queue = BIO_COMPLETION_QUEUE.get_with(&irq_guard);
        mem::swap(&mut *queue.borrow_mut(), &mut completion_queue);
    }

    while let Some(completion) = completion_queue.pop_front() {
        completion.bio.complete_now(completion.status);
    }
}
