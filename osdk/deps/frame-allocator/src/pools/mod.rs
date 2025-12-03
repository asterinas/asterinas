// SPDX-License-Identifier: MPL-2.0

mod balancing;

use core::{
    alloc::Layout,
    cell::RefCell,
    ops::{DerefMut, Drop, Range},
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local,
    irq::DisabledLocalIrqGuard,
    leader_cpu_local,
    mm::Paddr,
    numa::{leader_cpu_of, leader_cpu_of_node, memory_ranges, num_cpus_in_node, NodeId},
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::chunk::{greater_order_of, lesser_order_of, size_of_order, split_to_chunks, BuddyOrder};

use super::set::BuddySet;

// The NUMA node pools.
leader_cpu_local! {
    /// Free buddies in the NUMA node of the leader CPU.
    static NODE_POOL: SpinLock<BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled> = SpinLock::new(BuddySet::new_empty());
    /// A snapshot of the total size of the free buddies in the NUMA node of the leader CPU, not precise.
    static NODE_POOL_SIZE: AtomicUsize = AtomicUsize::new(0);
}

cpu_local! {
    /// CPU-local free buddies.
    static LOCAL_POOL: RefCell<BuddySet<MAX_LOCAL_BUDDY_ORDER>> = RefCell::new(BuddySet::new_empty());
}

/// Maximum supported order of the buddy system.
///
/// i.e., it is the number of classes of free blocks. It determines the
/// maximum size of each allocation.
///
/// A maximum buddy order of 32 supports up to 4KiB*2^31 = 8 TiB of chunks.
const MAX_BUDDY_ORDER: BuddyOrder = 32;

/// Maximum supported order of the buddy system for CPU-local buddy system.
///
/// Since large blocks are rarely allocated, caching such blocks will lead
/// to much fragmentation.
///
/// Lock guards are also allocated on stack. We can limit the stack usage
/// for common paths in this way.
///
/// A maximum local buddy order of 18 supports up to 4KiB*2^17 = 512 MiB of
/// chunks.
const MAX_LOCAL_BUDDY_ORDER: BuddyOrder = 18;

/// Allocates frames from the pools of the local NUMA node.
pub(super) fn alloc(guard: &DisabledLocalIrqGuard, layout: Layout) -> Option<Paddr> {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut node_pool = OnDemandNodeLock::new(leader_cpu_of(guard.current_cpu()));

    let size_order = greater_order_of(layout.size());
    let align_order = greater_order_of(layout.align());
    let order = size_order.max(align_order);

    let mut chunk_addr = None;

    if order < MAX_LOCAL_BUDDY_ORDER {
        chunk_addr = local_pool.alloc_chunk(order);
    }

    // Fall back to the NUMA node's free lists if the local free lists are empty.
    if chunk_addr.is_none() {
        chunk_addr = node_pool.get().alloc_chunk(order);
    }

    // TODO: On memory pressure the NUMA node pool may be not enough. We may need
    // to merge all buddy chunks from the local pools to the NUMA node pool and
    // try again.

    // FIXME: Fall back to the other NUMA node's free lists if the current NUMA node's
    // free lists are empty.

    // TODO: On memory pressure all the NUMA node pools may be not enough. We may need
    // to alloc across NUMA nodes.

    // If the alignment order is larger than the size order, we need to split
    // the chunk and return the rest part back to the free lists.
    let allocated_size = size_of_order(order);
    if allocated_size > layout.size()
        && let Some(chunk_addr) = chunk_addr
    {
        do_dealloc(
            Some(&mut local_pool),
            &mut node_pool,
            [(chunk_addr + layout.size(), allocated_size - layout.size())].into_iter(),
        );
    }

    balancing::balance(local_pool.deref_mut(), &mut node_pool);

    chunk_addr
}

/// Deallocates frames to the pools of the local NUMA node.
pub(super) fn dealloc_to_local(
    guard: &DisabledLocalIrqGuard,
    segments: impl Iterator<Item = (Paddr, usize)>,
) {
    let local_pool_cell = LOCAL_POOL.get_with(guard);
    let mut local_pool = local_pool_cell.borrow_mut();
    let mut node_pool = OnDemandNodeLock::new(leader_cpu_of(guard.current_cpu()));

    do_dealloc(Some(&mut local_pool), &mut node_pool, segments);

    balancing::balance(local_pool.deref_mut(), &mut node_pool);
}

/// Deallocates one frame to the pools of a remote NUMA node.
pub(super) fn dealloc_to_remote(segments: impl Iterator<Item = (Paddr, usize)>, node_id: NodeId) {
    let mut node_pool = OnDemandNodeLock::new(leader_cpu_of_node(node_id));

    do_dealloc(None, &mut node_pool, segments);
}

pub(super) fn add_free_memory(_guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    if size == 0 {
        return;
    }
    let mut free_memory = addr..addr + size;

    let add_free_memory_to_node = |range: &Range<usize>, node_id: NodeId| {
        let mut node_pool = OnDemandNodeLock::new(leader_cpu_of_node(node_id));

        split_to_chunks(range.start, range.end - range.start).for_each(|(addr, order)| {
            node_pool.get().insert_chunk(addr, order);
        });
    };

    for mem_range in memory_ranges()
        .iter()
        .filter(|mem_range| mem_range.is_enabled && mem_range.proximity_domain.is_some())
    {
        let range =
            mem_range.base_address as usize..(mem_range.base_address + mem_range.length) as usize;
        if range.start >= free_memory.end {
            break;
        }
        if range.end <= free_memory.start {
            continue;
        }

        // If the free memory does not belong to any NUMA node, assign it to
        // the default NUMA node (node 0).
        if free_memory.start < range.start {
            let non_overlap = free_memory.start..range.start.min(free_memory.end);
            add_free_memory_to_node(&non_overlap, NodeId::new(0));
            free_memory.start = non_overlap.end;
            if free_memory.is_empty() {
                break;
            }
        }

        let overlap = free_memory.start.max(range.start)..free_memory.end.min(range.end);
        let node_id = NodeId::new(mem_range.proximity_domain.unwrap());
        add_free_memory_to_node(&overlap, node_id);
        free_memory.start = overlap.end;
        if free_memory.is_empty() {
            break;
        }
    }

    if !free_memory.is_empty() {
        add_free_memory_to_node(&free_memory, NodeId::new(0));
    }
}

fn do_dealloc(
    mut local_pool: Option<&mut BuddySet<MAX_LOCAL_BUDDY_ORDER>>,
    node_pool: &mut OnDemandNodeLock,
    segments: impl Iterator<Item = (Paddr, usize)>,
) {
    segments.for_each(|(addr, size)| {
        split_to_chunks(addr, size).for_each(|(addr, order)| {
            if order < MAX_LOCAL_BUDDY_ORDER
                && let Some(local_pool) = local_pool.as_mut()
            {
                local_pool.insert_chunk(addr, order);
            } else {
                node_pool.get().insert_chunk(addr, order);
            }
        });
    });
}

type NodeLockGuard = SpinLockGuard<'static, BuddySet<MAX_BUDDY_ORDER>, LocalIrqDisabled>;

/// An on-demand guard that locks the NUMA node pool when needed.
///
/// It helps to avoid unnecessarily locking the node pool, and also avoids
/// repeatedly locking the node pool when it is needed multiple times.
struct OnDemandNodeLock {
    leader_cpu: CpuId,
    guard: Option<NodeLockGuard>,
}

impl OnDemandNodeLock {
    fn new(leader_cpu: CpuId) -> Self {
        Self {
            leader_cpu,
            guard: None,
        }
    }

    fn get(&mut self) -> &mut NodeLockGuard {
        self.guard
            .get_or_insert_with(|| node_pool(self.leader_cpu).lock())
    }

    /// Returns the size of the NUMA node pool.
    ///
    /// If the node pool is locked, returns the actual size of the node pool.
    /// Otherwise, returns the last snapshot of the node pool size by loading
    /// [`NODE_POOL_SIZE`].
    fn get_node_size(&self) -> usize {
        if let Some(guard) = self.guard.as_ref() {
            guard.total_size()
        } else {
            node_pool_size(self.leader_cpu).load(Ordering::Relaxed)
        }
    }

    /// Returns the number of CPUs in the NUMA node of the leader CPU.
    fn get_num_cpus_in_node(&self) -> usize {
        *num_cpus_in_node(self.leader_cpu).get().unwrap()
    }
}

impl Drop for OnDemandNodeLock {
    fn drop(&mut self) {
        // Updates the [`NODE_POOL_SIZE`] if the node pool is locked.
        if let Some(guard) = self.guard.as_ref() {
            node_pool_size(self.leader_cpu).store(guard.total_size(), Ordering::Relaxed);
        }
    }
}
