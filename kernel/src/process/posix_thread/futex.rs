// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;
use ostd::{
    cpu::num_cpus,
    sync::{Waiter, Waker},
};
use spin::Once;

use crate::{prelude::*, process::Pid, time::wait::ManagedTimeout};

type FutexBitSet = u32;

const FUTEX_OP_MASK: u32 = 0x0000_000F;
const FUTEX_FLAGS_MASK: u32 = 0xFFFF_FFF0;
const FUTEX_BITSET_MATCH_ANY: FutexBitSet = 0xFFFF_FFFF;

/// do futex wait
pub fn futex_wait(
    futex_addr: u64,
    futex_val: i32,
    timeout: Option<ManagedTimeout>,
    ctx: &Context,
    pid: Option<Pid>,
) -> Result<()> {
    futex_wait_bitset(
        futex_addr as _,
        futex_val,
        timeout,
        FUTEX_BITSET_MATCH_ANY,
        ctx,
        pid,
    )
}

/// Does futex wait bitset
pub fn futex_wait_bitset(
    futex_addr: Vaddr,
    futex_val: i32,
    timeout: Option<ManagedTimeout>,
    bitset: FutexBitSet,
    ctx: &Context,
    pid: Option<Pid>,
) -> Result<()> {
    debug!(
        "futex_wait_bitset addr: {:#x}, val: {}, bitset: {:#x}",
        futex_addr, futex_val, bitset
    );

    if bitset == 0 {
        return_errno_with_message!(Errno::EINVAL, "at least one bit should be set");
    }

    let futex_key = FutexKey::new(futex_addr, bitset, pid);
    let (futex_item, waiter) = FutexItem::create(futex_key);

    let (_, futex_bucket_ref) = get_futex_bucket(futex_key);
    // lock futex bucket ref here to avoid data race
    let mut futex_bucket = futex_bucket_ref.lock();

    if !futex_key.load_val(ctx).is_ok_and(|val| val == futex_val) {
        return_errno_with_message!(
            Errno::EAGAIN,
            "futex value does not match or load_val failed"
        );
    }

    futex_bucket.add_item(futex_item);

    // drop lock
    drop(futex_bucket);

    let result = waiter.pause_timeout(&timeout.into());
    match result {
        // FIXME: If the futex is woken up and a signal comes at the same time, we should succeed
        // instead of failing with `EINTR`. The code below is of course wrong, but was needed to
        // make the gVisor tests happy. See <https://github.com/asterinas/asterinas/pull/1577>.
        Err(err) if err.error() == Errno::EINTR => Ok(()),
        res => res,
    }

    // TODO: Ensure the futex item is dequeued and dropped.
    //
    // The enqueued futex item remain undequeued
    // if the futex wait operation is interrupted by a signal or times out.
    // In such cases, the `Box<FutexItem>` would persist in memory,
    // leaving our implementation vulnerable to exploitation by user programs
    // that could repeatedly issue futex wait operations
    // to exhaust kernel memory.
}

/// Does futex wake
pub fn futex_wake(futex_addr: Vaddr, max_count: usize, pid: Option<Pid>) -> Result<usize> {
    futex_wake_bitset(futex_addr, max_count, FUTEX_BITSET_MATCH_ANY, pid)
}

/// Does futex wake with bitset
pub fn futex_wake_bitset(
    futex_addr: Vaddr,
    max_count: usize,
    bitset: FutexBitSet,
    pid: Option<Pid>,
) -> Result<usize> {
    debug!(
        "futex_wake_bitset addr: {:#x}, max_count: {}, bitset: {:#x}",
        futex_addr, max_count, bitset
    );

    if bitset == 0 {
        return_errno_with_message!(Errno::EINVAL, "at least one bit should be set");
    }

    let futex_key = FutexKey::new(futex_addr, bitset, pid);
    let (_, futex_bucket_ref) = get_futex_bucket(futex_key);
    let mut futex_bucket = futex_bucket_ref.lock();
    let res = futex_bucket.remove_and_wake_items(futex_key, max_count);

    Ok(res)
}

/// This struct encodes the operation and comparison that are to be performed during
/// the futex operation with `FUTEX_WAKE_OP`.
///
/// The encoding is as follows:
///
/// +---+---+-----------+-----------+
/// |op |cmp|   oparg   |  cmparg   |
/// +---+---+-----------+-----------+
///   4   4       12          12    <== # of bits
///
/// Reference: https://man7.org/linux/man-pages/man2/futex.2.html.
struct FutexWakeOpEncode {
    op: FutexWakeOp,
    /// A flag indicating that the operation will use `1 << oparg`
    /// as the operand instead of `oparg` when it is set `true`.
    ///
    /// e.g. With this flag, [`FutexWakeOp::FUTEX_OP_ADD`] will be interpreted
    /// as `res = (1 << oparg) + oldval`.
    is_oparg_shift: bool,
    cmp: FutexWakeCmp,
    oparg: u32,
    cmparg: u32,
}

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(u32)]
#[expect(non_camel_case_types)]
enum FutexWakeOp {
    /// Calculate `res = oparg`.
    FUTEX_OP_SET = 0,
    /// Calculate `res = oparg + oldval`.
    FUTEX_OP_ADD = 1,
    /// Calculate `res = oparg | oldval`.
    FUTEX_OP_OR = 2,
    /// Calculate `res = oparg & !oldval`.
    FUTEX_OP_ANDN = 3,
    /// Calculate `res = oparg ^ oldval`.
    FUTEX_OP_XOR = 4,
}

#[derive(Debug, Copy, Clone, TryFromInt, PartialEq)]
#[repr(u32)]
#[expect(non_camel_case_types)]
enum FutexWakeCmp {
    /// If (oldval == cmparg) do wake.
    FUTEX_OP_CMP_EQ = 0,
    /// If (oldval != cmparg) do wake.
    FUTEX_OP_CMP_NE = 1,
    /// If (oldval < cmparg) do wake.
    FUTEX_OP_CMP_LT = 2,
    /// If (oldval <= cmparg) do wake.
    FUTEX_OP_CMP_LE = 3,
    /// If (oldval > cmparg) do wake.
    FUTEX_OP_CMP_GT = 4,
    /// If (oldval >= cmparg) do wake.
    FUTEX_OP_CMP_GE = 5,
}

impl FutexWakeOpEncode {
    fn from_u32(bits: u32) -> Result<Self> {
        let is_oparg_shift = (bits >> 31) & 1 == 1;
        let op = FutexWakeOp::try_from((bits >> 28) & 0x7)?;
        let cmp = FutexWakeCmp::try_from((bits >> 24) & 0xf)?;
        let oparg = (bits >> 12) & 0xfff;
        let cmparg = bits & 0xfff;

        Ok(FutexWakeOpEncode {
            op,
            is_oparg_shift,
            cmp,
            oparg,
            cmparg,
        })
    }

    fn calculate_new_val(&self, old_val: u32) -> u32 {
        let oparg = if self.is_oparg_shift {
            if self.oparg > 31 {
                // Linux might return EINVAL in the future
                // Reference: https://elixir.bootlin.com/linux/v6.15.2/source/kernel/futex/waitwake.c#L211-L222
                warn!("futex_wake_op: program tries to shift op by {}", self.oparg);
            }

            1 << (self.oparg & 31)
        } else {
            self.oparg
        };

        match self.op {
            FutexWakeOp::FUTEX_OP_SET => oparg,
            FutexWakeOp::FUTEX_OP_ADD => oparg.wrapping_add(old_val),
            FutexWakeOp::FUTEX_OP_OR => oparg | old_val,
            FutexWakeOp::FUTEX_OP_ANDN => oparg & !old_val,
            FutexWakeOp::FUTEX_OP_XOR => oparg ^ old_val,
        }
    }

    fn should_wake(&self, old_val: u32) -> bool {
        match self.cmp {
            FutexWakeCmp::FUTEX_OP_CMP_EQ => old_val == self.cmparg,
            FutexWakeCmp::FUTEX_OP_CMP_NE => old_val != self.cmparg,
            FutexWakeCmp::FUTEX_OP_CMP_LT => old_val < self.cmparg,
            FutexWakeCmp::FUTEX_OP_CMP_LE => old_val <= self.cmparg,
            FutexWakeCmp::FUTEX_OP_CMP_GT => old_val > self.cmparg,
            FutexWakeCmp::FUTEX_OP_CMP_GE => old_val >= self.cmparg,
        }
    }
}

pub fn futex_wake_op(
    futex_addr_1: Vaddr,
    futex_addr_2: Vaddr,
    max_count_1: usize,
    max_count_2: usize,
    wake_op_bits: u32,
    ctx: &Context,
    pid: Option<Pid>,
) -> Result<usize> {
    let wake_op = FutexWakeOpEncode::from_u32(wake_op_bits)?;

    let futex_key_1 = FutexKey::new(futex_addr_1, FUTEX_BITSET_MATCH_ANY, pid);
    let futex_key_2 = FutexKey::new(futex_addr_2, FUTEX_BITSET_MATCH_ANY, pid);
    let (index_1, futex_bucket_ref_1) = get_futex_bucket(futex_key_1);
    let (index_2, futex_bucket_ref_2) = get_futex_bucket(futex_key_2);

    let (mut futex_bucket_1, mut futex_bucket_2) = if index_1 == index_2 {
        (futex_bucket_ref_1.lock(), None)
    } else {
        // Ensure that we always lock the buckets in a consistent order to avoid deadlocks.
        if index_1 < index_2 {
            let bucket_1 = futex_bucket_ref_1.lock();
            let bucket_2 = futex_bucket_ref_2.lock();
            (bucket_1, Some(bucket_2))
        } else {
            let bucket_2 = futex_bucket_ref_2.lock();
            let bucket_1 = futex_bucket_ref_1.lock();
            (bucket_1, Some(bucket_2))
        }
    };

    // FIXME: This should be an atomic read-modify-write memory access here.
    let old_val = ctx.user_space().read_val(futex_addr_2)?;
    let new_val = wake_op.calculate_new_val(old_val);
    ctx.user_space().write_val(futex_addr_2, &new_val)?;

    let mut res = futex_bucket_1.remove_and_wake_items(futex_key_1, max_count_1);
    if wake_op.should_wake(old_val) {
        let bucket = futex_bucket_2.as_mut().unwrap_or(&mut futex_bucket_1);
        res += bucket.remove_and_wake_items(futex_key_2, max_count_2);
    }

    Ok(res)
}

/// Does futex requeue
pub fn futex_requeue(
    futex_addr: Vaddr,
    max_nwakes: usize,
    max_nrequeues: usize,
    futex_new_addr: Vaddr,
    pid: Option<Pid>,
) -> Result<usize> {
    if futex_new_addr == futex_addr {
        return futex_wake(futex_addr, max_nwakes, pid);
    }

    let futex_key = FutexKey::new(futex_addr, FUTEX_BITSET_MATCH_ANY, pid);
    let futex_new_key = FutexKey::new(futex_new_addr, FUTEX_BITSET_MATCH_ANY, pid);
    let (bucket_idx, futex_bucket_ref) = get_futex_bucket(futex_key);
    let (new_bucket_idx, futex_new_bucket_ref) = get_futex_bucket(futex_new_key);

    let nwakes = {
        if bucket_idx == new_bucket_idx {
            let mut futex_bucket = futex_bucket_ref.lock();
            let nwakes = futex_bucket.remove_and_wake_items(futex_key, max_nwakes);
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

            let nwakes = futex_bucket.remove_and_wake_items(futex_key, max_nwakes);
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
    ((1 << 8) * num_cpus()).next_power_of_two()
}

fn get_futex_bucket(key: FutexKey) -> (usize, &'static SpinLock<FutexBucket>) {
    FUTEX_BUCKETS.get().unwrap().get_bucket(key)
}

/// Initialize the futex system.
pub fn init() {
    FUTEX_BUCKETS.call_once(|| FutexBucketVec::new(get_bucket_count()));
}

struct FutexBucketVec {
    vec: Vec<SpinLock<FutexBucket>>,
}

impl FutexBucketVec {
    pub fn new(size: usize) -> FutexBucketVec {
        let mut buckets = FutexBucketVec {
            vec: Vec::with_capacity(size),
        };
        for _ in 0..size {
            let bucket = SpinLock::new(FutexBucket::new());
            buckets.vec.push(bucket);
        }
        buckets
    }

    pub fn get_bucket(&self, key: FutexKey) -> (usize, &SpinLock<FutexBucket>) {
        let index = (self.size() - 1) & {
            // The addr is the multiples of 4, so we ignore the last 2 bits
            let addr = key.addr() >> 2;
            // simple hash
            addr / self.size()
        };
        (index, &self.vec[index])
    }

    fn size(&self) -> usize {
        self.vec.len()
    }
}

struct FutexBucket {
    items: Vec<FutexItem>,
}

impl FutexBucket {
    pub fn new() -> FutexBucket {
        FutexBucket {
            items: Vec::with_capacity(1),
        }
    }

    pub fn add_item(&mut self, item: FutexItem) {
        self.items.push(item);
    }

    pub fn remove_and_wake_items(&mut self, key: FutexKey, max_count: usize) -> usize {
        let mut count = 0;

        self.items.retain(|item| {
            if item.key.match_up(&key) && count < max_count {
                if item.wake() {
                    count += 1;
                }
                false
            } else {
                true
            }
        });

        count
    }

    pub fn update_item_keys(&mut self, key: FutexKey, new_key: FutexKey, max_count: usize) {
        let mut count = 0;
        for item in self.items.iter_mut() {
            if item.key.match_up(&key) {
                item.key = new_key;
                count += 1;
            }
            if count >= max_count {
                break;
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
        self.items
            .extract_if(.., |item| {
                if item.key.match_up(&key) && count < max_nrequeues {
                    count += 1;
                    true
                } else {
                    false
                }
            })
            .for_each(|mut extracted| {
                extracted.key = new_key;
                another.add_item(extracted);
            });
    }
}

struct FutexItem {
    key: FutexKey,
    waker: Arc<Waker>,
}

impl FutexItem {
    pub fn create(key: FutexKey) -> (Self, Waiter) {
        let (waiter, waker) = Waiter::new_pair();
        let futex_item = FutexItem { key, waker };

        (futex_item, waiter)
    }

    #[must_use]
    pub fn wake(&self) -> bool {
        self.waker.wake_up()
    }
}

// The addr of a futex, it should be used to mark different futex word
#[derive(Debug, Clone, Copy)]
struct FutexKey {
    addr: Vaddr,
    bitset: FutexBitSet,
    /// Specify whether this `FutexKey` is process private or shared. If `pid` is
    /// None, then this `FutexKey` is shared.
    pid: Option<Pid>,
}

impl FutexKey {
    pub fn new(addr: Vaddr, bitset: FutexBitSet, pid: Option<Pid>) -> Self {
        Self { addr, bitset, pid }
    }

    pub fn load_val(&self, ctx: &Context) -> Result<i32> {
        // FIXME: how to implement a atomic load?
        warn!("implement an atomic load");
        ctx.user_space().read_val(self.addr)
    }

    pub fn addr(&self) -> Vaddr {
        self.addr
    }

    pub fn match_up(&self, another: &Self) -> bool {
        // TODO: Use hash value to do match_up
        self.addr == another.addr && (self.bitset & another.bitset) != 0 && self.pid == another.pid
    }
}

// The implementation is from occlum

#[derive(PartialEq, Debug, Clone, Copy)]
#[expect(non_camel_case_types)]
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
