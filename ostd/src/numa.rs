// SPDX-License-Identifier: MPL-2.0

//! NUMA(Non Uniform Memory Access) support.

use spin::Once;

use crate::{
    arch::boot::numa::{init_numa_topology, MemoryRange, MEMORY_RANGES, PROCESSOR_AFFINITIES},
    cpu::{all_cpus, CpuId},
    cpu_local,
    util::id_set::Id,
};

/// The number of NUMA nodes.
static mut NUM_NODES: usize = 0;

/// Initializes the number of NUMA nodes.
///
/// # Safety
///
/// The caller must ensure that we're in the boot context, and this method is
/// called only once.
unsafe fn init_num_nodes(num_nodes: usize) {
    assert!(num_nodes >= 1);

    // SAFETY: It is safe to mutate this global variable because we
    // are in the boot context.
    unsafe { NUM_NODES = num_nodes };
}

/// Returns the number of NUMA nodes.
pub fn num_nodes() -> usize {
    // SAFETY: As far as the safe APIs are concerned, `NUM_NODES` is initialized
    // and read-only, so it is always valid to read.
    unsafe { NUM_NODES }
}

cpu_local! {
    /// The NUMA node ID of the current CPU.
    static NODE_ID: Once<NodeId> = Once::new();
}

/// Returns the NUMA node ID of the given CPU.
///
/// # Panics
///
/// This method panics if the NUMA topology is not initialized.
pub fn node_id_of_cpu(cpu_id: CpuId) -> NodeId {
    debug_assert!(NODE_ID.get_on_cpu(cpu_id).is_completed());

    // SAFETY: As far as the safe APIs are concerned, `NODE_ID` has been initialized.
    unsafe { *NODE_ID.get_on_cpu(cpu_id).get_unchecked() }
}

pub(super) fn init() {
    let (num_nodes, leader_cpu) = init_numa_topology();
    log::warn!("NUMA node(s): {}", num_nodes);

    // SAFETY: We're in the boot context, calling the method only once.
    unsafe { init_num_nodes(num_nodes) };

    for affinity in PROCESSOR_AFFINITIES.get().unwrap().iter() {
        if !affinity.is_enabled {
            continue;
        }
        let node_id = affinity.proximity_domain;
        let cpu_id = CpuId::try_from(affinity.local_apic_id as usize).unwrap();
        NODE_ID
            .get_on_cpu(cpu_id)
            .call_once(|| NodeId::new(node_id));
    }
    for cpu_id in all_cpus() {
        NODE_ID.get_on_cpu(cpu_id).call_once(|| NodeId::new(0));
    }
}

/// The ID of a NUMA node in the system.
///
/// If converting from/to an integer, the integer must start from 0 and be less
/// than the number of NUMA nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeId(u32);

impl NodeId {
    /// Creates a new instance.
    ///
    /// # Panics
    ///
    /// The given number must be smaller than the total number of NUMA nodes
    /// (`ostd::numa::num_nodes()`).
    pub fn new(raw_id: u32) -> Self {
        assert!(raw_id < num_nodes() as u32);
        // SAFETY: The raw ID is smaller than `num_nodes()`.
        unsafe { Self::new_unchecked(raw_id) }
    }
}

impl From<NodeId> for u32 {
    fn from(node_id: NodeId) -> Self {
        node_id.0
    }
}

// SAFETY: `NodeId`s and the integers within 0 to `num_nodes` (exclusive) have 1:1 mapping.
unsafe impl Id for NodeId {
    unsafe fn new_unchecked(raw_id: u32) -> Self {
        Self(raw_id)
    }

    fn cardinality() -> u32 {
        num_nodes() as u32
    }
}

/// Returns the memory ranges associated with different proximity domains.
///
/// The ranges are sorted by starting physical address and do not overlap.
///
/// # Panics
///
/// This method panics if the NUMA topology is not initialized.
pub fn memory_ranges() -> &'static [MemoryRange] {
    MEMORY_RANGES.get().unwrap()
}
