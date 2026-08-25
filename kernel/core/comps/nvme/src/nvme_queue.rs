// SPDX-License-Identifier: MPL-2.0

//! NVMe Submission and Completion Queue implementation.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.3 (Queue Mechanism).

use alloc::vec::Vec;
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

/// Submission Queue with per-slot outstanding contexts.
///
/// Each slot's command identifier equals its index. `items[cid]` tracks the driver context
/// until the matching completion arrives.
pub(crate) struct NvmeSubmissionQueue<T> {
    inner: NvmeSubmissionQueueInner,
    items: [Option<T>; QUEUE_DEPTH],
}

impl<T> core::fmt::Debug for NvmeSubmissionQueue<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmeSubmissionQueue")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<T> NvmeSubmissionQueue<T> {
    /// Creates a new submission queue.
    ///
    /// Returns `None` if DMA memory for the submission ring cannot be allocated.
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            inner: NvmeSubmissionQueueInner::new()?,
            items: core::array::from_fn(|_| None),
        })
    }

    /// Updates the mirrored SQ head from the SQ head pointer in `completion`.
    pub(crate) fn update_sq_head(&mut self, completion: &NvmeCompletion) {
        self.inner.update_sq_head(completion);
    }

    /// Updates the mirrored SQ head from a raw head index reported by the controller.
    pub(crate) fn set_sq_head(&mut self, head: u16) {
        self.inner.set_sq_head(head);
    }

    /// Returns the DMA physical address of the submission ring.
    pub(crate) fn sq_daddr(&self) -> usize {
        self.inner.sq_daddr()
    }

    /// Returns the number of SQ slots available for new commands starting at the current tail.
    pub(crate) fn free_slots(&self) -> usize {
        let used = if self.inner.tail >= self.inner.head {
            (self.inner.tail - self.inner.head) as usize
        } else {
            QUEUE_DEPTH - (self.inner.head - self.inner.tail) as usize
        };
        (QUEUE_DEPTH - 1).saturating_sub(used)
    }

    /// Removes and returns the outstanding context for `cid`.
    ///
    /// Returns `None` if `cid` is out of range or the slot has no outstanding context.
    pub(crate) fn take_item(&mut self, cid: u16) -> Option<T> {
        self.items.get_mut(cid as usize)?.take()
    }
}

/// Hardware submission ring.
#[derive(Debug)]
pub(crate) struct NvmeSubmissionQueueInner {
    squeue: SafePtr<Sqring, DmaCoherent>,
    tail: u16,
    head: u16,
}

struct Sqring {
    ring: [NvmeCommand; QUEUE_DEPTH],
}

impl NvmeSubmissionQueueInner {
    /// Creates a new submission ring.
    ///
    /// Returns `None` if DMA memory for the submission ring cannot be allocated.
    fn new() -> Option<Self> {
        let dma = DmaCoherent::alloc(1, true).ok()?;
        Some(Self {
            squeue: SafePtr::new(dma, 0),
            tail: 0,
            head: 0,
        })
    }

    /// Updates the mirrored SQ head from the SQ head pointer in `completion`.
    pub(crate) fn update_sq_head(&mut self, completion: &NvmeCompletion) {
        self.set_sq_head(completion.sq_head());
    }

    /// Updates the mirrored SQ head from a raw head index reported by the controller.
    pub(crate) fn set_sq_head(&mut self, head: u16) {
        self.head = head % (QUEUE_DEPTH as u16);
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

    /// Consumes all ready completions and updates the CQ head doorbell once.
    ///
    /// Returns an empty vector when no completion is ready.
    pub(crate) fn complete(&mut self) -> Vec<NvmeCompletion> {
        let mut entries = Vec::new();
        let mut last_head = None;
        while let Some((entry, new_head)) = self.poll() {
            last_head = Some(new_head);
            entries.push(entry);
        }
        if let Some(new_head) = last_head {
            self.ring_doorbell(new_head);
        }
        entries
    }

    /// Polls one completion without ringing the CQ doorbell.
    fn poll(&mut self) -> Option<(NvmeCompletion, u16)> {
        let (new_head, entry) = self.queue.complete()?;
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
        Some((entry, new_head))
    }

    /// Rings the CQ head doorbell after one or more [`Self::poll`] calls.
    fn ring_doorbell(&mut self, new_head: u16) {
        // Full barrier: do not update the doorbell until the completion entry reads finish.
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Cqhdbl,
            self.qid,
            self.dstrd,
            new_head as u32,
        );
    }
}

pub(crate) struct NvmeSubmissionQueueAccess<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q, T> NvmeSubmissionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue<T>>,
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

    /// Submits a batch of commands with contexts and rings the SQ doorbell once.
    pub(crate) fn submit_with_items(
        &mut self,
        commands: impl IntoIterator<Item = (NvmeCommand, T)>,
    ) -> Option<usize> {
        let mut count = 0;
        for (entry, item) in commands {
            self.enqueue_one(entry, item)?;
            count += 1;
        }
        if count > 0 {
            self.ring_doorbell();
        }
        Some(count)
    }

    /// Returns the number of free SQ slots available for new commands.
    pub(crate) fn free_slots(&self) -> usize {
        self.queue.free_slots()
    }

    /// Submits a command and rings the SQ tail doorbell.
    ///
    /// Returns the command identifier used (same as the tail before enqueue), or `None` if the
    /// queue is full.
    pub(crate) fn submit(&mut self, mut entry: NvmeCommand) -> Option<u16> {
        let cid = self.queue.inner.tail;
        entry.set_cid(cid);
        self.queue.inner.submit(entry)?;
        self.ring_doorbell();
        Some(cid)
    }

    /// Enqueues one command and context.
    fn enqueue_one(&mut self, mut entry: NvmeCommand, item: T) -> Option<u16> {
        let cid = self.queue.inner.tail;
        if self.queue.items[cid as usize].is_some() {
            warn!(
                "submission queue {} slot {} is still outstanding",
                self.qid, cid
            );
            return None;
        }
        entry.set_cid(cid);
        self.queue.inner.submit(entry)?;
        self.queue.items[cid as usize] = Some(item);
        Some(cid)
    }

    fn ring_doorbell(&mut self) {
        let new_tail = self.queue.inner.tail;
        // Write barrier: do not update the doorbell until the submit entry writes finish.
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Sqtdbl,
            self.qid,
            self.dstrd,
            new_tail as u32,
        );
    }
}
