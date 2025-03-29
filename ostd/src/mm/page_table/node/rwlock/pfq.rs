// SPDX-License-Identifier: MPL-2.0

//! PFQ readers-writer lock.

use alloc::boxed::Box;
use core::{
    cell::SyncUnsafeCell,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use super::super::mcs;

#[derive(Debug)]
#[repr(C)]
pub struct PfqRwLock {
    rin: AtomicU32,
    rout: AtomicU32,
    last: AtomicU32,

    mcs_lock: mcs::LockBody,
    mcs_head: SyncUnsafeCell<Option<Pin<Box<mcs::Node>>>>,

    writer_blocking_readers: [AtomicBool; 2],
}

const READER_INCREMENT: u32 = 0x100;

const PHASE_BIT: u32 = 0x001;
const WRITER_PRESENT: u32 = 0x002;

const WRITER_MASK: u32 = PHASE_BIT | WRITER_PRESENT;

impl PfqRwLock {
    pub const fn new() -> Self {
        Self {
            rin: AtomicU32::new(0),
            rout: AtomicU32::new(0),
            last: AtomicU32::new(0),
            writer_blocking_readers: [AtomicBool::new(false), AtomicBool::new(false)],

            mcs_lock: mcs::LockBody::new(),
            mcs_head: SyncUnsafeCell::new(None),
        }
    }

    pub fn lock_read(&self) {
        let ticket = self.rin.fetch_add(READER_INCREMENT, Ordering::AcqRel);

        if ticket & WRITER_PRESENT != 0 {
            let phase = ticket & PHASE_BIT;
            while self.writer_blocking_readers[phase as usize].load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn unlock_read(&self) {
        let ticket = self.rout.fetch_add(READER_INCREMENT, Ordering::AcqRel);

        if ticket & WRITER_PRESENT != 0 && ticket == self.last.load(Ordering::Acquire) {
            // Wake up the writer.
            // SAFETY: We are the last reader and no writers will write this field.
            let head = unsafe { &*self.mcs_head.get() };
            let head = head
                .as_ref()
                .expect("There isn't writer but WRITER_PRESENT");
            unsafe { head.as_ref().wake_up() };
        }
    }

    pub fn lock_write(&self) {
        let node = Box::pin(mcs::Node::new());

        unsafe { node.as_ref().lock(&self.mcs_lock) };

        // SAFETY: We are the writer and no readers will read it for now.
        let old_head = unsafe { self.mcs_head.get().replace(Some(node)) };
        assert!(old_head.is_none(), "There is already a writer");
        let node_ref = unsafe { &*self.mcs_head.get() }.as_ref().unwrap().as_ref();

        let phase = self.rin.load(Ordering::Relaxed) & PHASE_BIT;
        self.writer_blocking_readers[phase as usize].store(true, Ordering::Release);

        let in_ = self.rin.fetch_or(WRITER_PRESENT, Ordering::AcqRel);
        self.last.store(
            in_.wrapping_sub(READER_INCREMENT) | WRITER_PRESENT,
            Ordering::Relaxed,
        );

        let out = self.rout.fetch_or(WRITER_PRESENT, Ordering::AcqRel);
        if in_ != out {
            // There are active readers.
            while unsafe { node_ref.is_blocked() } {
                core::hint::spin_loop();
            }
        }
    }

    pub fn unlock_write(&self) {
        let phase = self.rin.fetch_xor(WRITER_MASK, Ordering::Relaxed) & PHASE_BIT;
        self.rout.fetch_xor(WRITER_MASK, Ordering::Relaxed);

        self.writer_blocking_readers[phase as usize].store(false, Ordering::Release);

        let head = unsafe { self.mcs_head.get().replace(None) }.expect("There is no writer");

        unsafe { head.as_ref().unlock(&self.mcs_lock) };
    }
}
