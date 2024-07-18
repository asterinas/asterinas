// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{cpu::num_cpus, sync::WaitQueue};
use spin::Once;

use crate::{prelude::*, thread::Tid, util::read_val_from_user};

type FutexBitSet = u32;
type FutexBucketRef = Arc<Mutex<FutexBucket>>;

const FUTEX_OP_MASK: u32 = 0x0000_000F;
const FUTEX_FLAGS_MASK: u32 = 0xFFFF_FFF0;
const FUTEX_BITSET_MATCH_ANY: FutexBitSet = 0xFFFF_FFFF;

/// do futex wait
pub fn futex_wait(futex_addr: u64, futex_val: i32, timeout: &Option<FutexTimeout>) -> Result<()> {
    futex_wait_bitset(futex_addr as _, futex_val, timeout, FUTEX_BITSET_MATCH_ANY)
}

/// do futex wait bitset
pub fn futex_wait_bitset(
    futex_addr: Vaddr,
    futex_val: i32,
    timeout: &Option<FutexTimeout>,
    bitset: FutexBitSet,
) -> Result<()> {
    debug!(
        "futex_wait_bitset addr: {:#x}, val: {}, timeout: {:?}, bitset: {:#x}",
        futex_addr, futex_val, timeout, bitset
    );
    let futex_key = FutexKey::new(futex_addr);
    let (_, futex_bucket_ref) = get_futex_bucket(futex_key);

    // lock futex bucket ref here to avoid data race
    let mut futex_bucket = futex_bucket_ref.lock();

    if futex_key.load_val() != futex_val {
        return_errno_with_message!(Errno::EINVAL, "futex value does not match");
    }
    let futex_item = FutexItem::new(futex_key, bitset);
    futex_bucket.enqueue_item(futex_item.clone());

    // drop lock
    drop(futex_bucket);
    // Wait on the futex item
    futex_item.wait(timeout.clone());

    Ok(())
}

/// do futex wake
pub fn futex_wake(futex_addr: Vaddr, max_count: usize) -> Result<usize> {
    futex_wake_bitset(futex_addr, max_count, FUTEX_BITSET_MATCH_ANY)
}

/// Do futex wake with bitset
pub fn futex_wake_bitset(
    futex_addr: Vaddr,
    max_count: usize,
    bitset: FutexBitSet,
) -> Result<usize> {
    debug!(
        "futex_wake_bitset addr: {:#x}, max_count: {}, bitset: {:#x}",
        futex_addr, max_count, bitset
    );

    let futex_key = FutexKey::new(futex_addr);
    let (_, futex_bucket_ref) = get_futex_bucket(futex_key);
    let mut futex_bucket = futex_bucket_ref.lock();
    let res = futex_bucket.dequeue_and_wake_items(futex_key, max_count, bitset);
    drop(futex_bucket);

    Ok(res)
}

/// Do futex requeue
pub fn futex_requeue(
    futex_addr: Vaddr,
    max_nwakes: usize,
    max_nrequeues: usize,
    futex_new_addr: Vaddr,
) -> Result<usize> {
    if futex_new_addr == futex_addr {
        return futex_wake(futex_addr, max_nwakes);
    }

    let futex_key = FutexKey::new(futex_addr);
    let futex_new_key = FutexKey::new(futex_new_addr);
    let (bucket_idx, futex_bucket_ref) = get_futex_bucket(futex_key);
    let (new_bucket_idx, futex_new_bucket_ref) = get_futex_bucket(futex_new_key);

    let nwakes = {
        if bucket_idx == new_bucket_idx {
            let mut futex_bucket = futex_bucket_ref.lock();
            let nwakes =
                futex_bucket.dequeue_and_wake_items(futex_key, max_nwakes, FUTEX_BITSET_MATCH_ANY);
            futex_bucket.update_item_keys(futex_key, futex_new_key, max_nrequeues);
            drop(futex_bucket);
            nwakes
        } else {
            let (mut futex_bucket, mut futex_new_bucket) = {
                if bucket_idx < new_bucket_idx {
                    let futex_bucket = futex_bucket_ref.lock();
                    let futext_new_bucket = futex_new_bucket_ref.lock();
                    (futex_bucket, futext_new_bucket)
                } else {
                    // bucket_idx > new_bucket_idx
                    let futex_new_bucket = futex_new_bucket_ref.lock();
                    let futex_bucket = futex_bucket_ref.lock();
                    (futex_bucket, futex_new_bucket)
                }
            };

            let nwakes =
                futex_bucket.dequeue_and_wake_items(futex_key, max_nwakes, FUTEX_BITSET_MATCH_ANY);
            futex_bucket.requeue_items_to_another_bucket(
                futex_key,
                &mut futex_new_bucket,
                futex_new_key,
                max_nrequeues,
            );
            nwakes
        }
    };
    Ok(nwakes)
}

static FUTEX_BUCKETS: Once<FutexBucketVec> = Once::new();

/// Get the futex hash bucket count.
///
/// This number is calculated the same way as Linux's:
/// <https://github.com/torvalds/linux/blob/master/kernel/futex/core.c>
fn get_bucket_count() -> usize {
    ((1 << 8) * num_cpus()).next_power_of_two() as usize
}

fn get_futex_bucket(key: FutexKey) -> (usize, FutexBucketRef) {
    FUTEX_BUCKETS.get().unwrap().get_bucket(key)
}

/// Initialize the futex system.
pub fn init() {
    FUTEX_BUCKETS.call_once(|| FutexBucketVec::new(get_bucket_count()));
}

#[derive(Debug, Clone)]
pub struct FutexTimeout {}

impl FutexTimeout {
    pub fn new() -> Self {
        todo!()
    }
}

struct FutexBucketVec {
    vec: Vec<FutexBucketRef>,
}

impl FutexBucketVec {
    pub fn new(size: usize) -> FutexBucketVec {
        let mut buckets = FutexBucketVec {
            vec: Vec::with_capacity(size),
        };
        for _ in 0..size {
            let bucket = Arc::new(Mutex::new(FutexBucket::new()));
            buckets.vec.push(bucket);
        }
        buckets
    }

    pub fn get_bucket(&self, key: FutexKey) -> (usize, FutexBucketRef) {
        let index = (self.vec.len() - 1) & {
            // The addr is the multiples of 4, so we ignore the last 2 bits
            let addr = key.addr() >> 2;
            // simple hash
            addr / self.size()
        };
        (index, self.vec[index].clone())
    }

    fn size(&self) -> usize {
        self.vec.len()
    }
}

struct FutexBucket {
    queue: VecDeque<FutexItem>,
}

impl FutexBucket {
    pub fn new() -> FutexBucket {
        FutexBucket {
            queue: VecDeque::new(),
        }
    }

    pub fn enqueue_item(&mut self, item: FutexItem) {
        self.queue.push_back(item);
    }

    pub fn dequeue_item(&mut self, item: &FutexItem) {
        let item_i = self
            .queue
            .iter()
            .position(|futex_item| *futex_item == *item);
        if let Some(item_i) = item_i {
            self.queue.remove(item_i).unwrap();
        }
    }

    pub fn dequeue_and_wake_items(
        &mut self,
        key: FutexKey,
        max_count: usize,
        bitset: FutexBitSet,
    ) -> usize {
        let mut count = 0;
        let mut items_to_wake = Vec::new();

        self.queue.retain(|item| {
            if count >= max_count || key != item.key || (bitset & item.bitset) == 0 {
                true
            } else {
                items_to_wake.push(item.clone());
                count += 1;
                false
            }
        });

        FutexItem::batch_wake(&items_to_wake);
        count
    }

    pub fn update_item_keys(&mut self, key: FutexKey, new_key: FutexKey, max_count: usize) {
        let mut count = 0;
        for item in self.queue.iter_mut() {
            if count == max_count {
                break;
            }
            if item.key == key {
                item.key = new_key;
                count += 1;
            }
        }
    }

    pub fn requeue_items_to_another_bucket(
        &mut self,
        key: FutexKey,
        another: &mut Self,
        new_key: FutexKey,
        max_nrequeues: usize,
    ) {
        let mut count = 0;

        self.queue.retain(|item| {
            if count >= max_nrequeues || key != item.key {
                true
            } else {
                let mut new_item = item.clone();
                new_item.key = new_key;
                another.enqueue_item(new_item);
                count += 1;
                false
            }
        });
    }
}

#[derive(Debug, PartialEq, Clone)]
struct FutexItem {
    key: FutexKey,
    bitset: FutexBitSet,
    waiter: FutexWaiterRef,
}

impl FutexItem {
    pub fn new(key: FutexKey, bitset: FutexBitSet) -> Self {
        FutexItem {
            key,
            bitset,
            waiter: Arc::new(FutexWaiter::new()),
        }
    }

    pub fn wake(&self) {
        self.waiter.wake();
    }

    pub fn wait(&self, timeout: Option<FutexTimeout>) {
        self.waiter.wait(timeout);
    }

    pub fn waiter(&self) -> &FutexWaiterRef {
        &self.waiter
    }

    pub fn batch_wake(items: &[FutexItem]) {
        let waiters = items.iter().map(|item| item.waiter()).collect::<Vec<_>>();
        FutexWaiter::batch_wake(&waiters);
    }
}

// The addr of a futex, it should be used to mark different futex word
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FutexKey(Vaddr);

impl FutexKey {
    pub fn new(futex_addr: Vaddr) -> Self {
        FutexKey(futex_addr as _)
    }

    pub fn load_val(&self) -> i32 {
        // FIXME: how to implement a atomic load?
        warn!("implement an atomic load");
        read_val_from_user(self.0).unwrap()
    }

    pub fn addr(&self) -> Vaddr {
        self.0
    }
}

// The implementation is from occlum

#[derive(PartialEq, Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum FutexOp {
    FUTEX_WAIT = 0,
    FUTEX_WAKE = 1,
    FUTEX_FD = 2,
    FUTEX_REQUEUE = 3,
    FUTEX_CMP_REQUEUE = 4,
    FUTEX_WAKE_OP = 5,
    FUTEX_LOCK_PI = 6,
    FUTEX_UNLOCK_PI = 7,
    FUTEX_TRYLOCK_PI = 8,
    FUTEX_WAIT_BITSET = 9,
    FUTEX_WAKE_BITSET = 10,
}

impl FutexOp {
    pub fn from_u32(bits: u32) -> Result<FutexOp> {
        match bits {
            0 => Ok(FutexOp::FUTEX_WAIT),
            1 => Ok(FutexOp::FUTEX_WAKE),
            2 => Ok(FutexOp::FUTEX_FD),
            3 => Ok(FutexOp::FUTEX_REQUEUE),
            4 => Ok(FutexOp::FUTEX_CMP_REQUEUE),
            5 => Ok(FutexOp::FUTEX_WAKE_OP),
            6 => Ok(FutexOp::FUTEX_LOCK_PI),
            7 => Ok(FutexOp::FUTEX_UNLOCK_PI),
            8 => Ok(FutexOp::FUTEX_TRYLOCK_PI),
            9 => Ok(FutexOp::FUTEX_WAIT_BITSET),
            10 => Ok(FutexOp::FUTEX_WAKE_BITSET),
            _ => return_errno_with_message!(Errno::EINVAL, "Unknown futex op"),
        }
    }
}

bitflags! {
    pub struct FutexFlags : u32 {
        const FUTEX_PRIVATE         = 128;
        const FUTEX_CLOCK_REALTIME  = 256;
    }
}

impl FutexFlags {
    pub fn from_u32(bits: u32) -> Result<FutexFlags> {
        FutexFlags::from_bits(bits)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown futex flags"))
    }
}

pub fn futex_op_and_flags_from_u32(bits: u32) -> Result<(FutexOp, FutexFlags)> {
    let op = {
        let op_bits = bits & FUTEX_OP_MASK;
        FutexOp::from_u32(op_bits)?
    };
    let flags = {
        let flags_bits = bits & FUTEX_FLAGS_MASK;
        FutexFlags::from_u32(flags_bits)?
    };
    Ok((op, flags))
}

type FutexWaiterRef = Arc<FutexWaiter>;

struct FutexWaiter {
    is_woken: AtomicBool,
    wait_queue: WaitQueue,
    tid: Tid,
}

impl Debug for FutexWaiter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FutexWaiter")
            .field("is_woken", &self.is_woken)
            .field("tid", &self.tid)
            .finish()
    }
}

impl PartialEq for FutexWaiter {
    fn eq(&self, other: &Self) -> bool {
        self.tid == other.tid
    }
}

impl FutexWaiter {
    pub fn new() -> Self {
        Self {
            is_woken: AtomicBool::new(false),
            tid: current_thread!().tid(),
            wait_queue: WaitQueue::new(),
        }
    }

    pub fn wait(&self, timeout: Option<FutexTimeout>) {
        let current_thread = current_thread!();
        if current_thread.tid() != self.tid {
            return;
        }

        self.is_woken.store(false, Ordering::SeqCst);
        let wake_cond = || {
            if self.is_woken() {
                Some(())
            } else {
                None
            }
        };

        if let Some(_timeout) = timeout {
            todo!()
        } else {
            self.wait_queue.wait_until(wake_cond);
        }
    }

    pub fn wake(&self) {
        if !self.is_woken() {
            self.is_woken.store(true, Ordering::SeqCst);
            self.wait_queue.wake_all();
        }
    }

    pub fn is_woken(&self) -> bool {
        self.is_woken.load(Ordering::SeqCst)
    }

    pub fn batch_wake(waiters: &[&FutexWaiterRef]) {
        waiters.iter().for_each(|waiter| {
            waiter.wake();
        });
    }
}
