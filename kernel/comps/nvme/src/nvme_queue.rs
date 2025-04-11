// SPDX-License-Identifier: MPL-2.0

use core::{
    hint::spin_loop,
    sync::atomic::{Ordering, fence},
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::mm::{DmaCoherent, FrameAllocOptions};

use crate::nvme_cmd::{NVMeCommand, NVMeCompletion};

#[derive(Debug)]
pub enum NVMeQueueError {
    InvalidArgs,
    NotReady,
}

pub const QUEUE_DEPTH: usize = 64;
pub const QUEUE_NUM: usize = 2;

#[derive(Debug)]
pub struct NVMeCompletionQueue {
    cqueue: SafePtr<Cqring, DmaCoherent>,
    length: u32,
    head: u16,
    phase: bool,
}

struct Cqring {
    ring: [NVMeCompletion; QUEUE_DEPTH],
}

impl NVMeCompletionQueue {
    pub fn new() -> Result<Self, NVMeQueueError> {
        Ok(Self {
            cqueue: SafePtr::new(
                DmaCoherent::map(
                    FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                    true,
                )
                .unwrap(),
                0,
            ),
            length: QUEUE_DEPTH as u32,
            head: 0,
            phase: true,
        })
    }

    pub fn cq_paddr(&self) -> usize {
        self.cqueue.paddr()
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn complete(&mut self) -> Option<(u16, NVMeCompletion, u16)> {
        let head = self.head;
        let ring_ptr: SafePtr<[NVMeCompletion; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.cqueue, Cqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NVMeCompletion>();
        ring_slot_ptr.add(head as usize);
        let entry = ring_slot_ptr.read().unwrap();

        fence(Ordering::SeqCst);

        if ((entry.status & 1) == 1) == self.phase {
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

    pub fn complete_spin(&mut self) -> (u16, NVMeCompletion, u16) {
        loop {
            if let Some(some) = self.complete() {
                return some;
            } else {
                spin_loop();
            }
        }
    }
}

#[derive(Debug)]
pub struct NVMeSubmissionQueue {
    squeue: SafePtr<Sqring, DmaCoherent>,
    length: u32,
    tail: u16,
    head: u16,
}

struct Sqring {
    ring: [NVMeCommand; QUEUE_DEPTH],
}

impl NVMeSubmissionQueue {
    pub fn new() -> Result<Self, NVMeQueueError> {
        Ok(Self {
            squeue: SafePtr::new(
                DmaCoherent::map(
                    FrameAllocOptions::new().alloc_segment(1).unwrap().into(),
                    true,
                )
                .unwrap(),
                0,
            ),
            length: QUEUE_DEPTH as u32,
            tail: 0,
            head: 0,
        })
    }

    pub fn sq_paddr(&self) -> usize {
        self.squeue.paddr()
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn tail(&self) -> u16 {
        self.tail
    }

    fn is_empty(&self) -> bool {
        self.head == self.tail
    }
    fn is_full(&self) -> bool {
        self.head == self.tail + 1
    }

    pub fn submit(&mut self, mut entry: NVMeCommand) -> u16 {
        let tail = self.tail;
        entry.cid = tail;
        let ring_ptr: SafePtr<[NVMeCommand; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.squeue, Sqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NVMeCommand>();
        ring_slot_ptr.add(tail as usize);
        ring_slot_ptr.write(&entry).unwrap();

        fence(Ordering::SeqCst);

        self.tail = (tail + 1) % (self.length() as u16);
        self.tail
    }
}
