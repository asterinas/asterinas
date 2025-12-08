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
    /// The leader CPU of the current CPU, i.e., the CPU with the smallest ID
    /// in the current CPU's NUMA node.
    static LEADER_CPU: Once<CpuId> = Once::new();
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

/// Returns the leader CPU of the NUMA node of the given CPU.
///
/// # Panics
///
/// This method panics if the NUMA topology is not initialized.
pub fn leader_cpu_of(cpu_id: CpuId) -> CpuId {
    debug_assert!(LEADER_CPU.get_on_cpu(cpu_id).is_completed());

    // SAFETY: As far as the safe APIs are concerned, `LEADER_CPU` has been initialized.
    unsafe { *LEADER_CPU.get_on_cpu(cpu_id).get_unchecked() }
}
/// Defines a statically-allocated CPU-local variable, which is only meaningful
/// on leader CPUs, and automatically generates an accessor function for it.
///
/// # Examples
///
/// ```rust
/// leader_cpu_local! {
///     pub static FOO: AtomicU32 = AtomicU32::new(1);
/// }
/// ```
///
/// The code above will be expanded to:
///
/// ```rust
/// cpu_local! {
///     static FOO: AtomicU32 = AtomicU32::new(1);
/// }
///
/// pub fn foo(leader_cpu: CpuId) -> &'static AtomicU32 {
///   debug_assert!(is_leader_cpu(leader_cpu));
///   FOO.get_on_cpu(leader_cpu)
/// }
/// ```
///
/// # Panics
///
/// The accessor functions panic if the given CPU is not a leader CPU.
#[macro_export]
macro_rules! leader_cpu_local {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        cpu_local! {
            $(
                $(#[$attr])*
                static $name: $t = $init;
            )*
        }

        $(
            paste::paste! {
                $(#[$attr])*
                $vis fn [<$name:lower>](leader_cpu: CpuId) -> &'static $t {
                    debug_assert!(leader_cpu_of(leader_cpu) == leader_cpu);
                    $name.get_on_cpu(leader_cpu)
                }
            }
        )*
    };
}

leader_cpu_local! {
    /// The number of CPUs in the NUMA node of the leader CPU.
    pub static NUM_CPUS_IN_NODE: Once<usize> = Once::new();
}

static LEADER_CPU_OF_NODE: Once<&'static [CpuId]> = Once::new();

/// Returns the leader CPU of the given NUMA node.
///
/// # Panics
///
/// This method panics if the NUMA topology is not initialized.
pub fn leader_cpu_of_node(node_id: NodeId) -> CpuId {
    debug_assert!(LEADER_CPU_OF_NODE.is_completed());

    // SAFETY: As far as the safe APIs are concerned, `LEADER_CPU_OF_NODE` has been initialized.
    unsafe { LEADER_CPU_OF_NODE.get_unchecked()[node_id.as_usize()] }
}

pub(super) fn init() {
    let (num_nodes, leader_cpu_of_node) = init_numa_topology();
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

    // Initialize the leader CPU of each CPU. Since the number of CPUs won't be too large,
    // and `alloc` is not allowed now, the O(n^2) approach is suitable here.
    for cpu_id in all_cpus() {
        let leader_cpu = LEADER_CPU.get_on_cpu(cpu_id);
        if leader_cpu.is_completed() {
            continue;
        }
        leader_cpu.call_once(|| cpu_id);
        let node_id = node_id_of_cpu(cpu_id);
        leader_cpu_of_node[node_id.as_usize()] = cpu_id;
        let mut num_cpus_in_this_node = 1;

        all_cpus()
            .filter(|&id| id.as_usize() > cpu_id.as_usize() && node_id_of_cpu(id) == node_id)
            .for_each(|non_leader_id| {
                debug_assert!(!LEADER_CPU.get_on_cpu(non_leader_id).is_completed());
                num_cpus_in_this_node += 1;
                LEADER_CPU.get_on_cpu(non_leader_id).call_once(|| cpu_id);
            });
        num_cpus_in_node(cpu_id).call_once(|| num_cpus_in_this_node);
    }

    LEADER_CPU_OF_NODE.call_once(|| leader_cpu_of_node);

    // FIXME: Add a fall back list for each NUMA node.
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
