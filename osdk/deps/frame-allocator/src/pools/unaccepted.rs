// SPDX-License-Identifier: MPL-2.0

//! Management of unaccepted memory regions, including demand-based acceptance (refilling),
//! watermark-based threshold control, and synchronized access to the global unaccepted pool.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::chunk::{BuddyOrder, size_of_order};
use crate::pools::{
    GLOBAL_POOL, GLOBAL_POOL_SIZE, LocalIrqDisabled, MAX_BUDDY_ORDER, MAX_LOCAL_BUDDY_ORDER,
};
use crate::set::BuddySet;
use ostd::{
    mm::{Paddr, frame::AcceptError},
    sync::SpinLock,
};

/// The minimum order to allocate from the unaccepted pool.
pub(crate) const BITMAP_UNIT_ORDER: BuddyOrder = 9;

pub(crate) fn accept_memory_on_demand(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    alloc_order: BuddyOrder,
    force: bool,
) -> Result<(), AcceptError> {
    let unaccepted_size = UNACCEPTED_POOL_SIZE.load(Ordering::Relaxed);

    if unaccepted_size == 0 {
        return Ok(());
    }

    let total_free = crate::load_total_free_size();
    let accepted_free = total_free.saturating_sub(unaccepted_size);
    const WATERMARK_STEP: usize = 16 * 1024 * 1024;
    let low_watermark = (total_free * LOW_WATERMARK_PERMILLE / 1000).max(WATERMARK_STEP);

    if !force && accepted_free >= low_watermark {
        return Ok(());
    }

    if force {
        let request_order = alloc_order.max(BITMAP_UNIT_ORDER);
        accept_one_chunk(local_pool, request_order)?;
        return Ok(());
    } else {
        let target_watermark = low_watermark.saturating_add(WATERMARK_STEP);
        const MAX_ACCEPT_PER_RUN_BYTES: usize = 64 * 1024 * 1024;
        let mut bytes_to_accept = target_watermark
            .saturating_sub(accepted_free)
            .min(MAX_ACCEPT_PER_RUN_BYTES);

        while bytes_to_accept > 0 {
            match accept_one_chunk(local_pool, BITMAP_UNIT_ORDER)? {
                AcceptStatus::Done => {
                    let chunk_size = size_of_order(BITMAP_UNIT_ORDER);
                    bytes_to_accept = bytes_to_accept.saturating_sub(chunk_size);
                }
                AcceptStatus::Exhausted => break,
            }
        }
    }

    Ok(())
}

pub(crate) fn insert_unaccepted_chunk(addr: Paddr, order: BuddyOrder) {
    let mut unaccepted_pool = UNACCEPTED_POOL.lock();
    unaccepted_pool.insert_chunk(addr, order);
    UNACCEPTED_POOL_SIZE.store(unaccepted_pool.total_size(), Ordering::Release);
}

fn accept_one_chunk(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    request_order: BuddyOrder,
) -> Result<AcceptStatus, AcceptError> {
    let Some(addr) = alloc_unaccepted_chunk(request_order) else {
        return Ok(AcceptStatus::Exhausted);
    };
    let chunk_size = size_of_order(request_order);

    GLOBAL_ACCEPTING_SIZE.fetch_add(chunk_size, Ordering::SeqCst);

    let res = ostd::mm::frame::accept_unaccepted_memory(addr, chunk_size);
    if res.is_ok() {
        release_to_alloc_pool(local_pool, addr, request_order);
    } else {
        insert_unaccepted_chunk(addr, request_order);
    }

    GLOBAL_ACCEPTING_SIZE.fetch_sub(chunk_size, Ordering::SeqCst);

    res.map(|_| AcceptStatus::Done)
}

fn alloc_unaccepted_chunk(request_order: BuddyOrder) -> Option<Paddr> {
    let mut unaccepted_pool = UNACCEPTED_POOL.lock();

    if let Some(addr) = unaccepted_pool.alloc_chunk(request_order) {
        UNACCEPTED_POOL_SIZE.store(unaccepted_pool.total_size(), Ordering::Release);
        return Some(addr);
    }

    None
}

fn release_to_alloc_pool(
    local_pool: &mut BuddySet<MAX_LOCAL_BUDDY_ORDER>,
    addr: Paddr,
    order: BuddyOrder,
) {
    if order < MAX_LOCAL_BUDDY_ORDER {
        // Prefer current CPU after expensive on-demand accept.
        local_pool.insert_chunk(addr, order);
    } else {
        let mut global_pool = GLOBAL_POOL.lock();
        global_pool.insert_chunk(addr, order);
        GLOBAL_POOL_SIZE.store(global_pool.total_size(), Ordering::Release);
    }
}

#[derive(PartialEq)]
enum AcceptStatus {
    Done,
    Exhausted,
}

/// The percentage of total memory (in 1/1000 units) to use as the LOW watermark.
const LOW_WATERMARK_PERMILLE: usize = 15;
/// Global free buddies that are not accepted yet.
static UNACCEPTED_POOL: SpinLock<BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled> =
    SpinLock::new(BuddySet::new_empty());
/// A snapshot of the total size of the global unaccepted free buddies, not precise.
static UNACCEPTED_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_ACCEPTING_SIZE: AtomicUsize = AtomicUsize::new(0);
