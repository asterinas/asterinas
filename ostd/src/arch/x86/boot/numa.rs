// SPDX-License-Identifier: MPL-2.0

//! NUMA(Non Uniform Memory Access) boot support.

use core::{
    alloc::Layout,
    marker::{PhantomData, PhantomPinned},
    mem,
    pin::Pin,
};

use acpi::{
    sdt::{SdtHeader, Signature},
    AcpiTable,
};
use bitflags::bitflags;
use spin::Once;

use crate::{
    arch::kernel::acpi::get_acpi_tables,
    cpu::CpuId,
    mm::{frame::allocator, paddr_to_vaddr},
};

/// The processor affinity information for all CPUs.
pub(crate) static PROCESSOR_AFFINITIES: Once<&'static [ProcessorAffinity]> = Once::new();
/// The memory ranges associated with different proximity domains.
pub(crate) static MEMORY_RANGES: Once<&'static [MemoryRange]> = Once::new();
/// The NUMA distance matrix representing relative locality distances.
pub(crate) static DISTANCE_MATRIX: Once<&'static [u8]> = Once::new();

/// Initialize the NUMA topology, and returns the number of NUMA nodes, along with
/// a `[CpuId; num_nodes()]` array to store the leader cpu of each NUMA node.
pub(crate) fn init_numa_topology() -> (usize, &'static mut [CpuId]) {
    let (num_nodes, processor_affinities, memory_ranges) =
        parse_srat_table().unwrap_or((1, &mut [], &mut []));
    PROCESSOR_AFFINITIES.call_once(|| processor_affinities);
    MEMORY_RANGES.call_once(|| memory_ranges);

    let (num_nodes, (distance_matrix, leader_cpu)) = parse_slit_table(num_nodes);
    DISTANCE_MATRIX.call_once(|| distance_matrix);

    (num_nodes, leader_cpu)
}

/// Default distance within the same NUMA node, if not specified by SLIT.
const DEFAULT_LOCAL_DISTANCE: u8 = 10;
/// Default distance between different NUMA nodes, if not specified by SLIT.
const DEFAULT_REMOTE_DISTANCE: u8 = 20;

/// Parses the SRAT table and returns the number of NUMA nodes, along with
/// initialized processor affinity and memory range arrays.
///
/// Returns `None` if the SRAT table is missing.
fn parse_srat_table() -> Option<(
    usize,
    &'static mut [ProcessorAffinity],
    &'static mut [MemoryRange],
)> {
    let acpi_tables = get_acpi_tables()?;
    let srat_table = acpi_tables.find_table::<Srat>().ok()?;

    if srat_table
        .get()
        .entries()
        .any(|e| matches!(e, SratEntry::Gicc(_) | SratEntry::GicIts(_)))
    {
        unimplemented!("ARM SRAT entries are not supported");
    }

    let mut processor_count = 0;
    let mut memory_range_count = 0;
    let mut num_nodes = 0;

    for entry in srat_table.get().entries() {
        match entry {
            SratEntry::LocalApic(_) | SratEntry::LocalX2Apic(_) => processor_count += 1,
            SratEntry::Memory(_) => memory_range_count += 1,
            _ => (),
        }
    }

    // Allocate a region to store the information of processor affinities and memory ranges.
    let (processor_affinities, memory_ranges) = {
        let (layout, processor_affinities_offset, memory_ranges_offset) = {
            let processor_affinities_layout =
                Layout::array::<ProcessorAffinity>(processor_count).unwrap();
            let memory_ranges_layout = Layout::array::<MemoryRange>(memory_range_count).unwrap();
            let (layout, memory_ranges_offset) = processor_affinities_layout
                .extend(memory_ranges_layout)
                .unwrap();
            (layout, 0usize, memory_ranges_offset)
        };

        let paddr = allocator::early_alloc(layout).unwrap();
        let addr = paddr_to_vaddr(paddr);
        let processor_affinities_ptr =
            (addr + processor_affinities_offset) as *mut ProcessorAffinity;
        let memory_ranges_ptr = (addr + memory_ranges_offset) as *mut MemoryRange;

        // SAFETY: The memory is properly allocated. We exclusively own it. So it's valid to write.
        unsafe {
            for i in 0..processor_count {
                processor_affinities_ptr.add(i).write(ProcessorAffinity {
                    local_apic_id: 0,
                    proximity_domain: 0,
                    is_enabled: false,
                })
            }
            for i in 0..memory_range_count {
                memory_ranges_ptr.add(i).write(MemoryRange {
                    base_address: 0,
                    length: 0,
                    proximity_domain: None,
                    hot_pluggable: false,
                    non_volatile: false,
                    is_enabled: false,
                });
            }
        }

        // SAFETY: The memory is properly allocated and initialized. We exclusively own it. We
        // never deallocate it so it lives for `'static`. So we can create a mutable slice on it.
        unsafe {
            (
                core::slice::from_raw_parts_mut(processor_affinities_ptr, processor_count),
                core::slice::from_raw_parts_mut(memory_ranges_ptr, memory_range_count),
            )
        }
    };

    processor_count = 0;
    memory_range_count = 0;

    for entry in srat_table.get().entries() {
        match entry {
            SratEntry::LocalApic(entry) => {
                let local_apic_id = entry.apic_id as u32;
                let mut proximity_domain = entry.proximity_domain_low as u32;
                for i in 0..3 {
                    let shift = 8 * (3 - i);
                    proximity_domain += (entry.proximity_domain_high[i] as u32) << shift;
                }
                let flags = entry.flags;
                let is_enabled = flags.contains(LocalApicAffinityFlags::ENABLED);
                let processor_affinity = ProcessorAffinity {
                    local_apic_id,
                    proximity_domain,
                    is_enabled,
                };
                processor_affinities[processor_count] = processor_affinity;
                processor_count += 1;
                num_nodes = num_nodes.max((proximity_domain + 1) as usize);
            }
            SratEntry::LocalX2Apic(entry) => {
                let local_apic_id = entry.x2apic_id;
                let proximity_domain = entry.proximity_domain;
                let flags = entry.flags;
                let is_enabled = flags.contains(LocalX2ApicAffinityFlags::ENABLED);
                let processor_affinity = ProcessorAffinity {
                    local_apic_id,
                    proximity_domain,
                    is_enabled,
                };
                processor_affinities[processor_count] = processor_affinity;
                processor_count += 1;
                num_nodes = num_nodes.max((proximity_domain + 1) as usize);
            }
            SratEntry::Memory(entry) => {
                let flags = entry.flags;
                let base_address =
                    entry.base_address_low as u64 + ((entry.base_address_high as u64) << 32);
                let length = entry.length_low as u64 + ((entry.length_high as u64) << 32);
                let proximity_domain = Some(entry.proximity_domain);
                let hot_pluggable = flags.contains(MemoryAffinityFlags::HOT_PLUGGABLE);
                let non_volatile = flags.contains(MemoryAffinityFlags::NON_VOLATILE);
                let is_enabled = flags.contains(MemoryAffinityFlags::ENABLED);
                let memory_range = MemoryRange {
                    base_address,
                    length,
                    proximity_domain,
                    hot_pluggable,
                    non_volatile,
                    is_enabled,
                };
                memory_ranges[memory_range_count] = memory_range;
                memory_range_count += 1;
                num_nodes = num_nodes.max((entry.proximity_domain + 1) as usize);
            }
            // TODO: parse information of generic initiators
            SratEntry::GenericInitiator(_) => {}
            _ => {}
        }
    }

    // Sort the memory ranges by their base address.
    memory_ranges.sort_by_key(|memory_range| memory_range.base_address);

    // Remove overlapping memory ranges.
    let mut max_end = 0;

    for range in memory_ranges.iter_mut() {
        if range.base_address < max_end {
            log::warn!("Overlapping memory range detected: {:?}", range);
            let overlap = max_end - range.base_address;
            if overlap >= range.length {
                range.length = 0;
                range.is_enabled = false;
            } else {
                range.base_address += overlap;
                range.length -= overlap;
            }
        }
        let range_end = range.base_address.saturating_add(range.length);
        range.length = range_end - range.base_address;
        max_end = max_end.max(range_end);
    }

    Some((num_nodes, processor_affinities, memory_ranges))
}

/// Parses the SLIT table and returns the total number of NUMA nodes, along with the
/// initialized NUMA distance matrix, and a `[CpuId; num_nodes()]` array to store the
/// leader cpu of each NUMA node.
///
/// Falls back to default values if the SLIT table is missing.
fn parse_slit_table(num_nodes: usize) -> (usize, (&'static mut [u8], &'static mut [CpuId])) {
    let acpi_tables = get_acpi_tables().unwrap();
    let Some(slit_table) = acpi_tables.find_table::<Slit>().ok() else {
        return (num_nodes, default_distance_matrix(num_nodes));
    };

    let nr_system_localities = slit_table.get().nr_system_localities as usize;
    let num_nodes = num_nodes.max(nr_system_localities);
    let (distance_matrix, leader_cpu) = default_distance_matrix(num_nodes);

    for (idx, entry) in slit_table.get().entries().enumerate() {
        let (i, j) = (idx / nr_system_localities, idx % nr_system_localities);
        distance_matrix[i * num_nodes + j] = entry;
    }

    (num_nodes, (distance_matrix, leader_cpu))
}

/// Allocates and initializes a default NUMA distance matrix, along with a
/// `[CpuId; num_nodes()]` array to store the leader cpu of each NUMA node.
fn default_distance_matrix(num_nodes: usize) -> (&'static mut [u8], &'static mut [CpuId]) {
    let distance_count = num_nodes.checked_mul(num_nodes).unwrap();

    // Allocate a region to store the distance matrix.
    let (distance_matrix, leader_cpu) = {
        let (layout, distance_matrix_offset, leader_cpu_offset) = {
            let distance_matrix_layout = Layout::array::<u8>(distance_count).unwrap();
            let leader_cpu_layout = Layout::array::<CpuId>(num_nodes).unwrap();
            let (layout, leader_cpu_offset) =
                distance_matrix_layout.extend(leader_cpu_layout).unwrap();
            (layout, 0usize, leader_cpu_offset)
        };

        let paddr = allocator::early_alloc(layout).unwrap();
        let addr = paddr_to_vaddr(paddr);
        let distance_matrix_ptr = (addr + distance_matrix_offset) as *mut u8;
        let leader_cpu_ptr = (addr + leader_cpu_offset) as *mut CpuId;

        // SAFETY: The memory is properly allocated. We exclusively own it. So it's valid to write.
        unsafe {
            core::ptr::write_bytes(distance_matrix_ptr, 0, distance_count);
            for i in 0..num_nodes {
                leader_cpu_ptr.add(i).write(CpuId::bsp());
            }
        }

        // SAFETY: The memory is properly allocated and initialized. We exclusively own it. We
        // never deallocate it so it lives for `'static`. So we can create a mutable slice on it.
        unsafe {
            (
                core::slice::from_raw_parts_mut(distance_matrix_ptr, distance_count),
                core::slice::from_raw_parts_mut(leader_cpu_ptr, num_nodes),
            )
        }
    };

    for i in 0..num_nodes {
        for j in 0..num_nodes {
            distance_matrix[i * num_nodes + j] = if i == j {
                DEFAULT_LOCAL_DISTANCE
            } else {
                DEFAULT_REMOTE_DISTANCE
            };
        }
    }

    (distance_matrix, leader_cpu)
}

// TODO: Switch to the official `acpi` crate API once the PRs
// <https://github.com/rust-osdev/acpi/pull/241> and
// <https://github.com/rust-osdev/acpi/pull/243> are merged.

// *********** SRAT structures START ***********

/// System Resource Affinity Table (SRAT).
///
/// This optional table provides information that allows OSPM to associate the following types of
/// devices with system locality / proximity domains and clock domains:
/// - processors,
/// - memory ranges (including those provided by hot-added memory devices), and
/// - generic initiators (e.g. heterogeneous processors and accelerators, GPUs, and I/O devices
///   with integrated compute or DMA engines).
#[repr(C, packed)]
struct Srat {
    header: SdtHeader,
    _reserved_1: u32,
    _reserved_2: u64,
    _pinned: PhantomPinned,
}

/// ### Safety: Implementation properly represents a valid SRAT.
unsafe impl AcpiTable for Srat {
    const SIGNATURE: Signature = Signature::SRAT;

    fn header(&self) -> &SdtHeader {
        &self.header
    }
}

impl Srat {
    fn entries(self: Pin<&Self>) -> SratEntryIter {
        let ptr = unsafe { Pin::into_inner_unchecked(self) as *const Srat as *const u8 };
        SratEntryIter {
            pointer: unsafe { ptr.add(mem::size_of::<Srat>()) },
            remaining_length: self.header.length - mem::size_of::<Srat>() as u32,
            _phantom: PhantomData,
        }
    }
}

struct SratEntryIter<'a> {
    pointer: *const u8,
    remaining_length: u32,
    _phantom: PhantomData<&'a ()>,
}

impl<'a> Iterator for SratEntryIter<'a> {
    type Item = SratEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_length > 0 {
            let entry_pointer = self.pointer;
            let entry_header = unsafe { *(self.pointer as *const AffinityEntryHeader) };

            self.pointer = unsafe { self.pointer.offset(entry_header.length as isize) };
            self.remaining_length -= entry_header.length as u32;

            match entry_header.r#type {
                AffinityEntryType::LocalApic => Some(SratEntry::LocalApic(unsafe {
                    &*(entry_pointer as *const LocalApicAffinityEntry)
                })),
                AffinityEntryType::Memory => Some(SratEntry::Memory(unsafe {
                    &*(entry_pointer as *const MemoryAffinityEntry)
                })),
                AffinityEntryType::LocalX2Apic => Some(SratEntry::LocalX2Apic(unsafe {
                    &*(entry_pointer as *const LocalX2ApicAffinityEntry)
                })),
                AffinityEntryType::Gicc => Some(SratEntry::Gicc(unsafe {
                    &*(entry_pointer as *const GiccAffinityEntry)
                })),
                AffinityEntryType::GicIts => Some(SratEntry::GicIts(unsafe {
                    &*(entry_pointer as *const GicItsAffinityEntry)
                })),
                AffinityEntryType::GenericInitiator => Some(SratEntry::GenericInitiator(unsafe {
                    &*(entry_pointer as *const GenericInitiatorAffinityEntry)
                })),
            }
        } else {
            None
        }
    }
}

enum SratEntry<'a> {
    LocalApic(&'a LocalApicAffinityEntry),
    Memory(&'a MemoryAffinityEntry),
    LocalX2Apic(&'a LocalX2ApicAffinityEntry),
    // TODO: remove the attributes after we can parse these entries.
    #[expect(unused)]
    Gicc(&'a GiccAffinityEntry),
    #[expect(unused)]
    GicIts(&'a GicItsAffinityEntry),
    #[expect(unused)]
    GenericInitiator(&'a GenericInitiatorAffinityEntry),
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct AffinityEntryHeader {
    r#type: AffinityEntryType,
    length: u8,
}

#[expect(unused)]
#[derive(Clone, Copy)]
#[repr(u8)]
enum AffinityEntryType {
    LocalApic = 0,
    Memory = 1,
    LocalX2Apic = 2,
    Gicc = 3,
    GicIts = 4,
    GenericInitiator = 5,
}

#[repr(C, packed)]
struct LocalApicAffinityEntry {
    header: AffinityEntryHeader,
    proximity_domain_low: u8,
    apic_id: u8,
    flags: LocalApicAffinityFlags,
    local_sapic_eid: u8,
    proximity_domain_high: [u8; 3],
    clock_domain: u32,
}

bitflags! {
    struct LocalApicAffinityFlags: u32 {
        const ENABLED = 1;
    }
}

#[repr(C, packed)]
struct MemoryAffinityEntry {
    header: AffinityEntryHeader,
    proximity_domain: u32,
    _reserved_1: u16,
    base_address_low: u32,
    base_address_high: u32,
    length_low: u32,
    length_high: u32,
    _reserved_2: u32,
    flags: MemoryAffinityFlags,
    _reserved_3: u64,
}

bitflags! {
    struct MemoryAffinityFlags: u32 {
        const ENABLED = 1;
        const HOT_PLUGGABLE = 1 << 1;
        const NON_VOLATILE = 1 << 2;
    }
}

#[repr(C, packed)]
struct LocalX2ApicAffinityEntry {
    header: AffinityEntryHeader,
    _reserved_1: u16,
    proximity_domain: u32,
    x2apic_id: u32,
    flags: LocalX2ApicAffinityFlags,
    clock_domain: u32,
    _reserved_2: u32,
}

type LocalX2ApicAffinityFlags = LocalApicAffinityFlags;

#[repr(C, packed)]
struct GiccAffinityEntry {
    header: AffinityEntryHeader,
    proximity_domain: u32,
    acpi_processor_uid: u32,
    flags: GiccAffinityFlags,
    clock_domain: u32,
}

type GiccAffinityFlags = LocalApicAffinityFlags;

#[repr(C, packed)]
struct GicItsAffinityEntry {
    header: AffinityEntryHeader,
    proximity_domain: u32,
    _reserved: u16,
    its_id: u32,
}

#[repr(C, packed)]
struct GenericInitiatorAffinityEntry {
    header: AffinityEntryHeader,
    _reserved_1: u8,
    device_handle_type: DeviceHandleType,
    proximity_domain: u32,
    device_handle: DeviceHandle,
    flags: GenericInitiatorAffinityFlags,
    _reserved_2: u32,
}

#[expect(unused)]
#[repr(u8)]
#[non_exhaustive]
enum DeviceHandleType {
    Acpi = 0,
    Pci = 1,
}

// TODO: remove this attribute after we can parse generic initiator affinity entry.
#[expect(unused)]
#[repr(C)]
enum DeviceHandle {
    Acpi(AcpiDeviceHandle),
    Pci(PciDeviceHandle),
}

#[repr(C, packed)]
struct AcpiDeviceHandle {
    acpi_hid: u64,
    acpi_uid: u32,
    _reserved: u32,
}

#[repr(C, packed)]
struct PciDeviceHandle {
    pci_segment: u16,
    pci_bdf_number: u16,
    _reserved: [u32; 3],
}

bitflags! {
    struct GenericInitiatorAffinityFlags: u32 {
        const ENABLED = 1;
        const ARCHITECTURAL_TRANSACTIONS = 1 << 1;
    }
}

/// Processor affinity information.
#[derive(Debug, Clone)]
pub struct ProcessorAffinity {
    /// The processor's APIC ID.
    pub local_apic_id: u32,
    /// The processor's proximity domain (NUMA node) ID.
    pub proximity_domain: u32,
    /// Whether the processor is enabled.
    pub is_enabled: bool,
}

/// Memory range affinity information.
#[derive(Debug, Clone)]
pub struct MemoryRange {
    /// The starting physical address.
    pub base_address: u64,
    /// The size in bytes.
    pub length: u64,
    /// The NUMA proximity domain (node).
    pub proximity_domain: Option<u32>,
    /// Whether the memory range supports hot-plugging.
    pub hot_pluggable: bool,
    /// Whether the memory range is non-volatile.
    pub non_volatile: bool,
    /// Whether the memory range is enabled.
    pub is_enabled: bool,
}

// ************ SRAT structures END ************

// *********** SLIT structures START ***********

/// System Locality Information Table (SLIT)
///
/// This optional table provides a matrix that describes the relative distance
/// (memory latency) between all System Localities, which are also referred to
/// as Proximity Domains. The value of each Entry[i,j] in the SLIT table, where
/// i represents a row of a matrix and j represents a column of a matrix,
/// indicates the relative distances from System Locality / Proximity Domain i
/// to every other System Locality j in the system (including itself).
#[repr(C, packed)]
pub struct Slit {
    header: SdtHeader,
    nr_system_localities: u64,
    _pinned: PhantomPinned,
}

unsafe impl AcpiTable for Slit {
    const SIGNATURE: Signature = Signature::SLIT;

    fn header(&self) -> &SdtHeader {
        &self.header
    }
}

impl Slit {
    fn entries(self: Pin<&Self>) -> SlitEntryIter {
        let ptr = unsafe { Pin::into_inner_unchecked(self) as *const Slit as *const u8 };
        SlitEntryIter {
            pointer: unsafe { ptr.add(size_of::<Slit>()) },
            remaining_length: self.header.length - size_of::<Slit>() as u32,
        }
    }
}

struct SlitEntryIter {
    pointer: *const u8,
    remaining_length: u32,
}

impl Iterator for SlitEntryIter {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_length > 0 {
            let entry_pointer = self.pointer;
            let size_of_item = size_of::<Self::Item>();
            self.pointer = unsafe { self.pointer.add(size_of_item) };
            self.remaining_length -= size_of_item as u32;
            Some(unsafe { *entry_pointer })
        } else {
            None
        }
    }
}

// ************ SLIT structures END ************
