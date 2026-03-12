// SPDX-License-Identifier: MPL-2.0

//! NVMe Submission and Completion Queue implementation.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.3 (Queue Mechanism).

use core::{
    ops::DerefMut,
    sync::atomic::{Ordering, fence},
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::mm::{HasDaddr, dma::DmaCoherent};

use crate::{
    nvme_cmd::{NvmeCommand, NvmeCompletion, STATUS_PHASE_TAG_MASK},
    nvme_regs::NvmeDoorBellRegs,
    transport::pci::transport::DbregAccess,
};

#[derive(Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeQueueError {
    InvalidArgs,
    NotReady,
}

pub(crate) const QUEUE_DEPTH: usize = 64;
pub(crate) const QUEUE_NUM: usize = 2;

/// Completion Queue.
#[derive(Debug)]
pub(crate) struct NvmeCompletionQueue {
    cqueue: SafePtr<Cqring, DmaCoherent>,
    length: u32,
    head: u16,
    phase: bool,
}

pub(crate) struct Cqring {
    ring: [NvmeCompletion; QUEUE_DEPTH],
}

impl NvmeCompletionQueue {
    pub(crate) fn new() -> Result<Self, NvmeQueueError> {
        Ok(Self {
            cqueue: SafePtr::new(DmaCoherent::alloc_uninit(1, true).unwrap(), 0),
            length: QUEUE_DEPTH as u32,
            head: 0,
            phase: true,
        })
    }

    pub(crate) fn cq_daddr(&self) -> usize {
        self.cqueue.daddr()
    }

    pub(crate) fn length(&self) -> u32 {
        self.length
    }

    pub(crate) fn complete(&mut self) -> Option<(u16, NvmeCompletion)> {
        fence(Ordering::SeqCst);

        let head = self.head;
        let ring_ptr: SafePtr<[NvmeCompletion; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.cqueue, Cqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCompletion>();
        ring_slot_ptr.add(head as usize);
        let entry = ring_slot_ptr.read().unwrap();

        // Check Phase Tag to determine if this entry is valid
        if ((entry.status & STATUS_PHASE_TAG_MASK) == 1) == self.phase {
            self.head = (self.head + 1) % (self.length() as u16);
            if self.head == 0 {
                self.phase = !self.phase;
            }
            Some((self.head, entry))
        } else {
            None
        }
    }
}

/// Submission Queue.
#[derive(Debug)]
pub(crate) struct NvmeSubmissionQueue {
    squeue: SafePtr<Sqring, DmaCoherent>,
    length: u32,
    tail: u16,
}

pub(crate) struct Sqring {
    ring: [NvmeCommand; QUEUE_DEPTH],
}

impl NvmeSubmissionQueue {
    pub(crate) fn new() -> Result<Self, NvmeQueueError> {
        Ok(Self {
            squeue: SafePtr::new(DmaCoherent::alloc_uninit(1, true).unwrap(), 0),
            length: QUEUE_DEPTH as u32,
            tail: 0,
        })
    }

    pub(crate) fn sq_daddr(&self) -> usize {
        self.squeue.daddr()
    }

    pub(crate) fn length(&self) -> u32 {
        self.length
    }

    pub(crate) fn tail(&self) -> u16 {
        self.tail
    }

    pub(crate) fn submit(&mut self, entry: NvmeCommand) -> u16 {
        let tail = self.tail;
        let ring_ptr: SafePtr<[NvmeCommand; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.squeue, Sqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCommand>();
        ring_slot_ptr.add(tail as usize);
        ring_slot_ptr.write(&entry).unwrap();

        fence(Ordering::SeqCst);

        self.tail = (tail + 1) % (self.length() as u16);
        self.tail
    }
}

pub(crate) struct NvmeSubmissionQueueGuard<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q> NvmeSubmissionQueueGuard<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue>,
{
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    pub(crate) fn submit(&mut self, entry: NvmeCommand) -> u16 {
        let tail = self.queue.submit(entry);
        self.dbregs
            .write_racy(NvmeDoorBellRegs::Sqtdb, self.qid, self.dstrd, tail as u32);
        tail
    }
}

pub(crate) struct NvmeCompletionQueueGuard<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q> NvmeCompletionQueueGuard<'a, Q>
where
    Q: DerefMut<Target = NvmeCompletionQueue>,
{
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    pub(crate) fn complete(&mut self) -> Option<NvmeCompletion> {
        let (new_head, entry) = self.queue.complete()?;
        self.dbregs.write_racy(
            NvmeDoorBellRegs::Cqhdb,
            self.qid,
            self.dstrd,
            new_head as u32,
        );
        Some(entry)
    }
}
