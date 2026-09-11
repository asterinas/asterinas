// SPDX-License-Identifier: MPL-2.0

//! NVMe Submission and Completion Queue implementation.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.3 (Queue Mechanism).

use core::{
    ops::DerefMut,
    sync::atomic::{Ordering, fence},
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::{
    mm::{HasDaddr, dma::DmaCoherent},
    warn,
};

use crate::{
    nvme_regs::NvmeDoorbellRegs,
    nvme_spec::{NvmeCommand, NvmeCompletion},
    transport::pci::transport::DbregAccess,
};

/// Number of entries in each submission and completion ring.
pub(crate) const QUEUE_DEPTH: usize = 64;

/// Number of queue pairs the driver allocates (admin plus I/O).
//
// TODO: This value should be changed when supporting more than 1 I/O queue pairs.
pub(crate) const QUEUE_NUM: usize = 2;

/// Completion Queue.
#[derive(Debug)]
pub(crate) struct NvmeCompletionQueue {
    cqueue: SafePtr<Cqring, DmaCoherent>,
    head: u16,
    phase: bool,
}

struct Cqring {
    ring: [NvmeCompletion; QUEUE_DEPTH],
}

impl NvmeCompletionQueue {
    /// Creates a new completion ring.
    ///
    /// Returns `None` if DMA memory for the completion ring cannot be allocated.
    pub(crate) fn new() -> Option<Self> {
        let dma = DmaCoherent::alloc(1, true).ok()?;
        Some(Self {
            cqueue: SafePtr::new(dma, 0),
            head: 0,
            phase: true,
        })
    }

    /// Returns the DMA physical address of the completion ring.
    pub(crate) fn cq_daddr(&self) -> usize {
        self.cqueue.daddr()
    }

    /// Consumes the next completion entry if its phase tag matches the expected phase.
    ///
    /// Returns the new head index (for the CQ head doorbell) and the completion, or `None` if no
    /// entry is ready.
    fn complete(&mut self) -> Option<(u16, NvmeCompletion)> {
        let ring_ptr: SafePtr<[NvmeCompletion; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.cqueue, Cqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCompletion>();
        ring_slot_ptr.add(self.head as usize);

        let phase_tag = NvmeCompletion::read_phase_tag(&ring_slot_ptr);
        if phase_tag != self.phase {
            return None;
        }

        // Read barrier.
        fence(Ordering::SeqCst);

        let entry = ring_slot_ptr
            .read()
            .expect("CQ slot pointer must be valid within allocated DMA ring");
        self.head = (self.head + 1) % (QUEUE_DEPTH as u16);
        if self.head == 0 {
            self.phase = !self.phase;
        }
        Some((self.head, entry))
    }
}

/// Submission Queue.
#[derive(Debug)]
pub(crate) struct NvmeSubmissionQueue {
    squeue: SafePtr<Sqring, DmaCoherent>,
    tail: u16,
    head: u16,
}

struct Sqring {
    ring: [NvmeCommand; QUEUE_DEPTH],
}

impl NvmeSubmissionQueue {
    /// Creates a new submission ring.
    ///
    /// Returns `None` if DMA memory for the submission ring cannot be allocated.
    pub(crate) fn new() -> Option<Self> {
        let dma = DmaCoherent::alloc(1, true).ok()?;
        Some(Self {
            squeue: SafePtr::new(dma, 0),
            tail: 0,
            head: 0,
        })
    }

    /// Updates the mirrored SQ head from the SQ head pointer in `completion`.
    pub(crate) fn update_sq_head(&mut self, completion: &NvmeCompletion) {
        self.head = completion.sq_head() % (QUEUE_DEPTH as u16);
    }

    /// Returns the DMA physical address of the submission ring.
    pub(crate) fn sq_daddr(&self) -> usize {
        self.squeue.daddr()
    }

    /// Enqueues a command into the submission ring.
    ///
    /// Does nothing when the queue is full (`(tail + 1) % size == head`).
    ///
    /// Returns the new tail index for the SQ Tail doorbell, or `None` if full.
    fn submit(&mut self, entry: NvmeCommand) -> Option<u16> {
        let next_tail = (self.tail + 1) % (QUEUE_DEPTH as u16);
        if next_tail == self.head {
            return None;
        }

        let ring_ptr: SafePtr<[NvmeCommand; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.squeue, Sqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCommand>();
        ring_slot_ptr.add(self.tail as usize);
        ring_slot_ptr
            .write(&entry)
            .expect("SQ slot pointer must be valid within allocated DMA ring");

        self.tail = next_tail;
        Some(self.tail)
    }
}

pub(crate) struct NvmeCompletionQueueAccess<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q> NvmeCompletionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeCompletionQueue>,
{
    /// Binds queue `qid` and doorbell stride `dstrd` to `queue` and `dbregs` for locked poll.
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    /// Polls for a completion and updates the CQ head doorbell when an entry is consumed.
    pub(crate) fn complete(&mut self) -> Option<NvmeCompletion> {
        let (new_head, entry) = self.queue.complete()?;
        // Full barrier: do not update the doorbell until the completion entry read finishes.
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Cqhdbl,
            self.qid,
            self.dstrd,
            new_head as u32,
        );
        if entry.has_error() {
            warn!(
                "completion queue {}: command failed (CID={}, status={:04X}, SC={:#04x}, SQID={})",
                self.qid,
                entry.cid(),
                entry.status(),
                entry.status_code(),
                entry.sq_id(),
            );
        }
        Some(entry)
    }
}

pub(crate) struct NvmeSubmissionQueueAccess<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q> NvmeSubmissionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue>,
{
    /// Binds queue `qid` and doorbell stride `dstrd` to `queue` and `dbregs` for locked submit.
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    /// Submits a command and rings the SQ tail doorbell.
    ///
    /// Writes at the current tail, sets the command identifier to that slot index, advances the
    /// tail, then updates the doorbell.
    ///
    /// Returns the command identifier used (same as the tail before enqueue), or `None` if the
    /// queue is full.
    pub(crate) fn submit(&mut self, mut entry: NvmeCommand) -> Option<u16> {
        let cid = self.queue.tail;
        entry.set_cid(cid);
        let new_tail = match self.queue.submit(entry) {
            Some(tail) => tail,
            None => {
                warn!("submission queue {} is full", self.qid);
                return None;
            }
        };
        // Write barrier: do not update the doorbell until the submit entry write finishes.
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Sqtdbl,
            self.qid,
            self.dstrd,
            new_tail as u32,
        );
        Some(cid)
    }
}
