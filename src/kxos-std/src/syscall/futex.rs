use crate::{memory::read_val_from_user, syscall::SYS_FUTEX};

use super::SyscallResult;
use crate::prelude::*;
use kxos_frame::{cpu::num_cpus, sync::WaitQueue};

type FutexBitSet = u32;
type FutexBucketRef = Arc<Mutex<FutexBucket>>;

const FUTEX_OP_MASK: u32 = 0x0000_000F;
const FUTEX_FLAGS_MASK: u32 = 0xFFFF_FFF0;
const FUTEX_BITSET_MATCH_ANY: FutexBitSet = 0xFFFF_FFFF;

pub fn sys_futex(
    futex_addr: u64,
    futex_op: u64,
    futex_val: u64,
    utime_addr: u64,
    futex_new_addr: u64,
    bitset: u64,
) -> SyscallResult {
    debug!("[syscall][id={}][SYS_FUTEX]", SYS_FUTEX);
    // FIXME: we current ignore futex flags
    let (futex_op, futex_flags) = futex_op_and_flags_from_u32(futex_op as _).unwrap();

    let get_futex_val = |val: i32| -> Result<usize, &'static str> {
        if val < 0 {
            return Err("the futex val must not be negative");
        }
        Ok(val as usize)
    };

    let get_futex_timeout = |timeout_addr| -> Result<Option<FutexTimeout>, &'static str> {
        if timeout_addr == 0 {
            return Ok(None);
        }
        // TODO: parse a timeout
        todo!()
    };

    let res = match futex_op {
        FutexOp::FUTEX_WAIT => {
            let timeout = get_futex_timeout(utime_addr).expect("Invalid time addr");
            futex_wait(futex_addr as _, futex_val as _, &timeout).map(|_| 0)
        }
        FutexOp::FUTEX_WAIT_BITSET => {
            let timeout = get_futex_timeout(utime_addr).expect("Invalid time addr");
            futex_wait_bitset(futex_addr as _, futex_val as _, &timeout, bitset as _).map(|_| 0)
        }
        FutexOp::FUTEX_WAKE => {
            let max_count = get_futex_val(futex_val as i32).expect("Invalid futex val");
            futex_wake(futex_addr as _, max_count).map(|count| count as isize)
        }
        FutexOp::FUTEX_WAKE_BITSET => {
            let max_count = get_futex_val(futex_val as i32).expect("Invalid futex val");
            futex_wake_bitset(futex_addr as _, max_count, bitset as _).map(|count| count as isize)
        }
        FutexOp::FUTEX_REQUEUE => {
            let max_nwakes = get_futex_val(futex_val as i32).expect("Invalid futex val");
            let max_nrequeues = get_futex_val(utime_addr as i32).expect("Invalid utime addr");
            futex_requeue(
                futex_addr as _,
                max_nwakes,
                max_nrequeues,
                futex_new_addr as _,
            )
            .map(|nwakes| nwakes as _)
        }
        _ => panic!("Unsupported futex operations"),
    }
    .unwrap();

    SyscallResult::Return(res as _)
}

/// do futex wait
pub fn futex_wait(
    futex_addr: u64,
    futex_val: i32,
    timeout: &Option<FutexTimeout>,
) -> Result<(), &'static str> {
    futex_wait_bitset(futex_addr as _, futex_val, timeout, FUTEX_BITSET_MATCH_ANY)
}

/// do futex wait bitset
pub fn futex_wait_bitset(
    futex_addr: Vaddr,
    futex_val: i32,
    timeout: &Option<FutexTimeout>,
    bitset: FutexBitSet,
) -> Result<(), &'static str> {
    debug!(
        "futex_wait_bitset addr: {:#x}, val: {}, timeout: {:?}, bitset: {:#x}",
        futex_addr, futex_val, timeout, bitset
    );
    let futex_key = FutexKey::new(futex_addr);
    let (_, futex_bucket_ref) = FUTEX_BUCKETS.get_bucket(futex_key);

    // lock futex bucket ref here to avoid data race
    let futex_bucket = futex_bucket_ref.lock();

    if futex_key.load_val() != futex_val {
        return Err("futex value does not match");
    }
    let futex_item = FutexItem::new(futex_key, bitset);
    futex_bucket.enqueue_item(futex_item);

    let wait_queue = futex_bucket.wait_queue();

    // drop lock
    drop(futex_bucket);

    wait_queue.wait_on(futex_item);

    Ok(())
}

/// do futex wake
pub fn futex_wake(futex_addr: Vaddr, max_count: usize) -> Result<usize, &'static str> {
    futex_wake_bitset(futex_addr, max_count, FUTEX_BITSET_MATCH_ANY)
}

/// Do futex wake with bitset
pub fn futex_wake_bitset(
    futex_addr: Vaddr,
    max_count: usize,
    bitset: FutexBitSet,
) -> Result<usize, &'static str> {
    debug!(
        "futex_wake_bitset addr: {:#x}, max_count: {}, bitset: {:#x}",
        futex_addr as usize, max_count, bitset
    );

    let futex_key = FutexKey::new(futex_addr);
    let (_, futex_bucket_ref) = FUTEX_BUCKETS.get_bucket(futex_key);
    let futex_bucket = futex_bucket_ref.lock();
    let res = futex_bucket.batch_wake_and_deque_items(futex_key, max_count, bitset);
    Ok(res)
}

/// Do futex requeue
pub fn futex_requeue(
    futex_addr: Vaddr,
    max_nwakes: usize,
    max_nrequeues: usize,
    futex_new_addr: Vaddr,
) -> Result<usize, &'static str> {
    if futex_new_addr == futex_addr {
        return futex_wake(futex_addr, max_nwakes);
    }

    let futex_key = FutexKey::new(futex_addr);
    let futex_new_key = FutexKey::new(futex_new_addr);
    let (bucket_idx, futex_bucket_ref) = FUTEX_BUCKETS.get_bucket(futex_key);
    let (new_bucket_idx, futex_new_bucket_ref) = FUTEX_BUCKETS.get_bucket(futex_new_key);

    let nwakes = {
        if bucket_idx == new_bucket_idx {
            let futex_bucket = futex_bucket_ref.lock();
            let nwakes = futex_bucket.batch_wake_and_deque_items(
                futex_key,
                max_nwakes,
                FUTEX_BITSET_MATCH_ANY,
            );
            futex_bucket.update_item_keys(futex_key, futex_new_key, max_nrequeues);
            drop(futex_bucket);
            nwakes
        } else {
            let (futex_bucket, futex_new_bucket) = {
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

            let nwakes = futex_bucket.batch_wake_and_deque_items(
                futex_key,
                max_nwakes,
                FUTEX_BITSET_MATCH_ANY,
            );
            futex_bucket.requeue_items_to_another_bucket(
                futex_key,
                &futex_new_bucket,
                futex_new_key,
                max_nrequeues,
            );
            nwakes
        }
    };
    Ok(nwakes)
}

lazy_static! {
    // Use the same count as linux kernel to keep the same performance
    static ref BUCKET_COUNT: usize = ((1<<8)* num_cpus()).next_power_of_two() as _;
    static ref BUCKET_MASK: usize = *BUCKET_COUNT - 1;
    static ref FUTEX_BUCKETS: FutexBucketVec = FutexBucketVec::new(*BUCKET_COUNT);
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
        let index = *BUCKET_MASK & {
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
    wait_queue: Arc<WaitQueue<FutexItem>>,
}

impl FutexBucket {
    pub fn new() -> FutexBucket {
        FutexBucket {
            wait_queue: Arc::new(WaitQueue::new()),
        }
    }

    pub fn wait_queue(&self) -> Arc<WaitQueue<FutexItem>> {
        self.wait_queue.clone()
    }

    pub fn enqueue_item(&self, item: FutexItem) {
        self.wait_queue.enqueue(item);
    }

    pub fn dequeue_item(&self, item: FutexItem) {
        self.wait_queue.dequeue(item);
    }

    pub fn batch_wake_and_deque_items(
        &self,
        key: FutexKey,
        max_count: usize,
        bitset: FutexBitSet,
    ) -> usize {
        self.wait_queue.batch_wake_and_deque(
            max_count,
            &(key, bitset),
            |futex_item, (futex_key, bitset)| {
                if futex_item.key == *futex_key && (*bitset & futex_item.bitset) != 0 {
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn update_item_keys(&self, key: FutexKey, new_key: FutexKey, max_count: usize) {
        self.wait_queue.update_waiters_data(
            |futex_key, futex_item| futex_item.key == *futex_key,
            &key,
            &new_key,
            |futex_item, new_futex_key| FutexItem::new(new_futex_key.clone(), futex_item.bitset),
            max_count,
        )
    }

    pub fn requeue_items_to_another_bucket(
        &self,
        key: FutexKey,
        another: &Self,
        new_key: FutexKey,
        max_nrequeues: usize,
    ) {
        let requeue_items =
            self.wait_queue
                .remove_waiters(|item, key| item.key == *key, &key, max_nrequeues);

        requeue_items.into_iter().for_each(|mut item| {
            item.key = new_key;
            another.enqueue_item(item);
        });
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct FutexItem {
    key: FutexKey,
    bitset: FutexBitSet,
}

impl FutexItem {
    pub fn new(key: FutexKey, bitset: FutexBitSet) -> Self {
        FutexItem { key, bitset }
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
        read_val_from_user(self.0)
    }

    pub fn addr(&self) -> Vaddr {
        self.0
    }
}

// The implementation is from occlum

#[derive(PartialEq)]
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
    pub fn from_u32(bits: u32) -> Result<FutexOp, &'static str> {
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
            _ => Err("Unknown futex op"),
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
    pub fn from_u32(bits: u32) -> Result<FutexFlags, &'static str> {
        FutexFlags::from_bits(bits).ok_or_else(|| "unknown futex flags")
    }
}

pub fn futex_op_and_flags_from_u32(bits: u32) -> Result<(FutexOp, FutexFlags), &'static str> {
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
