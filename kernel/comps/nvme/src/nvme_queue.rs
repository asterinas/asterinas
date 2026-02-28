// SPDX-License-Identifier: MPL-2.0

//! NVMe Submission and Completion Queue implementation.
//!
//! Refer to NVM Express Base Specification Revision 2.0, Section 3.3 (Queue Mechanism).

use core::{
    hint::spin_loop,
    sync::atomic::{Ordering, fence},
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::mm::{HasDaddr, dma::DmaCoherent};

use crate::nvme_cmd::{NvmeCommand, NvmeCompletion, STATUS_PHASE_TAG_MASK};

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

    pub(crate) fn complete(&mut self) -> Option<(u16, NvmeCompletion, u16)> {
        fence(Ordering::SeqCst);

        let head = self.head;
        let ring_ptr: SafePtr<[NvmeCompletion; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.cqueue, Cqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCompletion>();
        ring_slot_ptr.add(head as usize);
        let entry = ring_slot_ptr.read().unwrap();

        // Check Phase Tag to determine if this entry is valid
        if ((entry.status & STATUS_PHASE_TAG_MASK) == 1) == self.phase {
            let old_head = self.head;
            self.head = (self.head + 1) % (self.length() as u16);
            if self.head == 0 {
                self.phase = !self.phase;
            }
            Some((self.head, entry, old_head))
        } else {
            None
        }
    }

    pub(crate) fn complete_spin(&mut self) -> (u16, NvmeCompletion, u16) {
        loop {
            if let Some(some) = self.complete() {
                return some;
            } else {
                spin_loop();
            }
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

    pub(crate) fn submit(&mut self, mut entry: NvmeCommand) -> u16 {
        let tail = self.tail;
        entry.cid = tail;
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
