// SPDX-License-Identifier: MPL-2.0
//
// This is the impl<'a, 'b>ementation of the MCS readers-writer (prefer readers) lock
// by John M. Mellor-Crummey and Michael L. Scott., published as:
//
// Mellor-Crummey, John M., and Michael L. Scott. "Scalable reader-writer
// synchronization for shared-memory multiprocessors." ACM SIGPLAN Notices
// 26.7 (1991): 106-113.

//! MCS readers-writer (prefer readers) spinlock algorithm.

use core::{
    marker::PhantomPinned,
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
};

#[derive(Debug)]
pub(crate) struct LockBody {
    reader_head: AtomicPtr<UnsafeNode>,
    writer_tail: AtomicPtr<UnsafeNode>,
    writer_head: AtomicPtr<UnsafeNode>,
    rdr_cnt_and_flags: AtomicU32,
}

// layout of rdr_cnt_and_flags:
//  31     ...      2       1              0
// +-----------------+-------------+-----------------+
// | interested rdrs | active wtr? | interested wtr? |
// +-----------------+-------------+-----------------+

const WRITER_INTERESTED: u32 = 0x1;
const WRITER_ACTIVE: u32 = 0x2;
const READER_COUNT: u32 = 0x4;

impl LockBody {
    pub(crate) const fn new() -> Self {
        Self {
            reader_head: AtomicPtr::new(core::ptr::null_mut()),
            writer_tail: AtomicPtr::new(core::ptr::null_mut()),
            writer_head: AtomicPtr::new(core::ptr::null_mut()),
            rdr_cnt_and_flags: AtomicU32::new(0),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnsafeNode {
    next: AtomicPtr<UnsafeNode>,
    blocked: AtomicBool,
    /// A phantom pinned marker.
    _pinned: PhantomPinned,
}

impl UnsafeNode {
    pub(crate) fn new_reader() -> Self {
        Self {
            next: AtomicPtr::new(core::ptr::null_mut()),
            blocked: AtomicBool::new(false),
            _pinned: PhantomPinned,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Node<'a, 'b, const IS_READER: bool, const READY: bool> {
    lock: &'a LockBody,
    node: Pin<&'b mut UnsafeNode>,
}

impl<'a, 'b> Node<'a, 'b, true, true> {
    pub(crate) fn new_reader(lock: &'a LockBody, node: Pin<&'b mut UnsafeNode>) -> Self {
        Self { lock, node }
    }

    pub(crate) fn lock_reader(self) -> Node<'a, 'b, true, false> {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();
        
        let rdr_cnt_and_flag = self
            .lock
            .rdr_cnt_and_flags
            .fetch_add(READER_COUNT, Ordering::AcqRel);

        let read_rdr_cnt_and_flag = || self
            .lock
            .rdr_cnt_and_flags
            .load(Ordering::Relaxed);

        if rdr_cnt_and_flag & WRITER_ACTIVE != 0 {
            self.node.blocked.store(true, Ordering::Relaxed);
            let next = self.lock.reader_head.swap(node_ptr, Ordering::AcqRel);
            if read_rdr_cnt_and_flag() & WRITER_ACTIVE == 0 {
                // Writer no longer active; wake any waiting readers.
                let head = self
                    .lock
                    .reader_head
                    .swap(core::ptr::null_mut(), Ordering::AcqRel);
                if !head.is_null() {
                    // SAFETY: Head is not null so it exists.
                    unsafe { &*head }.blocked.store(false, Ordering::Relaxed);
                }
            }
            while self.node.blocked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
            if !next.is_null() {
                // SAFETY: Next is not null so it exists.
                unsafe { &*next }.blocked.store(false, Ordering::Relaxed);
            }
        }

        Node::<true, false> {
            lock: self.lock,
            node: self.node,
        }
    }
}

impl<'a, 'b> Node<'a, 'b, true, false> {
    pub(crate) fn unlock_reader(self) -> Node<'a, 'b, true, true> {
        let rdr_cnt_and_flag = self
            .lock
            .rdr_cnt_and_flags
            .fetch_sub(READER_COUNT, Ordering::AcqRel);

        // If I am the last reader, resume the first waiting writer (if any).
        if rdr_cnt_and_flag == (READER_COUNT + WRITER_INTERESTED)
            && self.lock.rdr_cnt_and_flags.compare_exchange(
                WRITER_INTERESTED,
                WRITER_ACTIVE,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) == Ok(WRITER_INTERESTED)
        {
            let writer_head = self.lock.writer_head.load(Ordering::Acquire);
            if !writer_head.is_null() {
                // SAFETY: writer_head is not null so it exists.
                unsafe { &*writer_head }
                    .blocked
                    .store(false, Ordering::Relaxed);
            }
        }

        Node::<true, true> {
            lock: self.lock,
            node: self.node,
        }
    }
}

impl<'a, 'b> Node<'a, 'b, false, true> {
    pub(crate) fn new_writer(lock: &'a LockBody, node: Pin<&'b mut UnsafeNode>) -> Self {
        Self { lock, node }
    }

    pub(crate) fn lock_writer(self) -> Node<'a, 'b, false, false> {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();

        self.node.blocked.store(true, Ordering::Relaxed);
        let pred = self.lock.writer_tail.swap(node_ptr, Ordering::AcqRel);
        if pred.is_null() {
            self.lock.writer_head.store(node_ptr, Ordering::Relaxed);
            if self
                .lock
                .rdr_cnt_and_flags
                .fetch_or(WRITER_INTERESTED, Ordering::AcqRel)
                == 0
                && self.lock.rdr_cnt_and_flags.compare_exchange(
                    WRITER_INTERESTED,
                    WRITER_ACTIVE,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) == Ok(WRITER_INTERESTED)
            {
                return Node::<false, false> {
                    lock: self.lock,
                    node: self.node,
                };
            }
        } else {
            // SAFETY: pred is not null so it exists.
            unsafe { &*pred }.next.store(node_ptr, Ordering::Relaxed);
        }
        while self.node.blocked.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }

        Node::<false, false> {
            lock: self.lock,
            node: self.node,
        }
    }
}

impl<'a, 'b> Node<'a, 'b, false, false> {
    pub(crate) fn unlock_writer(self) -> Node<'a, 'b, false, true> {
        let node_ptr = (&*self.node as *const UnsafeNode).cast_mut();

        self.lock
            .writer_head
            .store(core::ptr::null_mut(), Ordering::Relaxed);
        // clear wtr flag and test for waiting readers
        if self
            .lock
            .rdr_cnt_and_flags
            .fetch_and(!WRITER_ACTIVE, Ordering::AcqRel)
            != 0
        {
            // waiting readers exist
            let head = self
                .lock
                .reader_head
                .swap(core::ptr::null_mut(), Ordering::AcqRel);
            if !head.is_null() {
                // SAFETY: head is not null so it exists.
                unsafe { &*head }.blocked.store(false, Ordering::Relaxed);
            }
        }
        // testing next is strictly an optimization
        if !self.node.next.load(Ordering::Relaxed).is_null()
            || self
                .lock
                .writer_tail
                .compare_exchange(
                    node_ptr,
                    core::ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            // resolve successor
            while self.node.next.load(Ordering::Relaxed).is_null() {
                core::hint::spin_loop();
            }
            self.lock
                .writer_head
                .store(self.node.next.load(Ordering::Relaxed), Ordering::Relaxed);
            if self
                .lock
                .rdr_cnt_and_flags
                .fetch_or(WRITER_INTERESTED, Ordering::AcqRel)
                == 0
                && self.lock.rdr_cnt_and_flags.compare_exchange(
                    WRITER_INTERESTED,
                    WRITER_ACTIVE,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) != Ok(0)
            {
                let writer_head = self.lock.writer_head.load(Ordering::Acquire);
                if !writer_head.is_null() {
                    // SAFETY: writer_head is not null so it exists.
                    unsafe { &*writer_head }
                        .blocked
                        .store(false, Ordering::Relaxed);
                }
            }
            // else readers will wake up the writer
        }

        Node::<false, true> {
            lock: self.lock,
            node: self.node,
        }
    }
}
