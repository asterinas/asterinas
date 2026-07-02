// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use alloc::vec::Vec;
use core::{alloc::Layout, ops::RangeInclusive};

use fdt::node::FdtNode;
use ostd::{
    Error,
    arch::{
        boot::DEVICE_TREE,
        irq::{IRQ_CHIP, InterruptSourceInFdt, MappedIrqLine},
    },
    io::IoMem,
    irq::IrqLine,
    mm::VmIoOnce,
    prelude::Paddr,
    sync::SpinLock,
    warn,
};
use spin::Once;

use crate::{PciDeviceLocation, cfg_space::PciCommonCfgOffset};

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();
static PCI_INTX_MAPPER: Once<PciIntxMapper> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | offset) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | offset) as usize)
}

/// The maximum offset in the 12-bit configuration space when using [`encode_as_address_offset`].
const PCI_ECAM_MAX_OFFSET: u32 = 0xffc;

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    // We only support ECAM here for RISC-V platforms. Offsets are from
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    ((location.bus as u32) << 20)
        | ((location.device as u32) << 15)
        | ((location.function as u32) << 12)
}

/// Initializes the platform-specific module for accessing the PCI configuration space.
///
/// Returns a range for the PCI bus number, or [`None`] if there is no PCI bus.
pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    // We follow the Linux's PCI device tree to obtain the register information
    // about the PCI bus. See also the specification at
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    //
    // TODO: Support multiple PCIe segment groups instead of assuming only one
    // PCIe segment group is in use.
    let Some(pci) = DEVICE_TREE
        .get()
        .unwrap()
        .find_compatible(&["pci-host-ecam-generic"])
    else {
        warn!("no generic host controller node found in the device tree");
        return None;
    };

    let Some(mut reg) = pci.reg() else {
        warn!("node should have exactly one `reg` property, but found zero `reg`s");
        return None;
    };
    let Some(region) = reg.next() else {
        warn!("node should have exactly one `reg` property, but found zero `reg`s");
        return None;
    };
    if reg.next().is_some() {
        warn!(
            "node should have exactly one `reg` property, but found {} `reg`s",
            reg.count() + 2
        );
        return None;
    }

    let bus_range = if let Some(prop) = pci.property("bus-range") {
        if prop.value.len() != 8 || prop.value[0..3] != [0, 0, 0] || prop.value[4..7] != [0, 0, 0] {
            warn!(
                "node should have a `bus-range` property with two bytes, but found `{:?}`",
                prop.value
            );
            return None;
        }
        if prop.value[3] != 0 {
            // TODO: We don't support this case because the base address corresponds to the first
            // bus. Therefore, an offset must be applied to the bus value in `read32`/`write32`.
            warn!(
                "node with a non-zero bus start `{}` is not supported yet",
                prop.value[3]
            );
            return None;
        }
        Some(prop.value[3]..=prop.value[7])
    } else {
        // "bus-range: Optional property [..] If absent, defaults to <0 255> (i.e. all buses)."
        Some(0..=255)
    };

    // Initialize the MMIO allocator used to assign base addresses to device BARs.
    // On RISC-V the firmware (OpenSBI) does not perform PCI enumeration, so the
    // OS must allocate BAR addresses within the host bridge's memory-mapped window.
    init_mmio_allocator_from_fdt(&pci);
    init_intx_mapper_from_fdt(&pci);

    let addr_start = region.starting_address as usize;
    let addr_end = addr_start.checked_add(region.size.unwrap()).unwrap();
    PCI_ECAM_CFG_SPACE.call_once(|| IoMem::acquire(addr_start..addr_end).unwrap());

    bus_range
}

/// A simple MMIO allocator managing a linear region.
///
/// RISC-V platforms booted via OpenSBI do not have firmware that performs PCI
/// enumeration, so the base addresses of PCI device BARs must be allocated by
/// the OS within the host bridge's memory-mapped window.
///
/// This is a bump allocator: addresses are handed out monotonically and never
/// reclaimed. That is intentional — BAR base addresses are assigned once during
/// enumeration and are not freed afterwards.
struct MmioAllocator {
    base: Paddr,
    size: Paddr,
    offset: Paddr,
}

impl MmioAllocator {
    /// Creates a new MMIO allocator with a given base and size.
    const fn new(base: Paddr, size: Paddr) -> Self {
        MmioAllocator {
            base,
            size,
            offset: 0,
        }
    }

    /// Allocates a physical address range with the specified alignment and size.
    fn allocate(&mut self, layout: Layout) -> Option<Paddr> {
        let (aligned, next_offset) = allocate_from_linear_region(
            self.base,
            self.size,
            self.offset,
            layout.align(),
            layout.size(),
        )?;

        self.offset = next_offset;
        Some(aligned)
    }
}

fn allocate_from_linear_region(
    base: Paddr,
    region_size: Paddr,
    offset: Paddr,
    align: usize,
    size: usize,
) -> Option<(Paddr, Paddr)> {
    let current = base.checked_add(offset)?;
    let align_mask = align.checked_sub(1)?;
    let aligned = current.checked_add(align_mask)? & !align_mask;
    let aligned_offset = aligned.checked_sub(base)?;
    let next_offset = aligned_offset.checked_add(size)?;

    if next_offset > region_size {
        return None;
    }

    Some((aligned, next_offset))
}

static MMIO_ALLOCATOR: Once<SpinLock<MmioAllocator>> = Once::new();

/// Initializes the MMIO allocator from the PCIe node's "ranges" property.
///
/// Only the first non-prefetchable, 32-bit Memory window (`phys.hi >> 24 == 0x2`)
/// is used; prefetchable and 64-bit windows, and any subsequent windows, are
/// ignored. All BARs (including prefetchable and 64-bit ones) are therefore
/// allocated from this single window.
fn init_mmio_allocator_from_fdt(node: &FdtNode) {
    let Some(ranges) = node.property("ranges") else {
        warn!("PCIe node has no `ranges` property");
        return;
    };
    let data = ranges.value;

    const RANGE_ENTRY_SIZE: usize = 7 * size_of::<u32>();
    for entry in data.chunks_exact(RANGE_ENTRY_SIZE) {
        let pci_space = u32::from_be_bytes(entry[0..4].try_into().unwrap());
        let _pci_addr = u64::from_be_bytes(entry[4..12].try_into().unwrap());
        let cpu_addr = u64::from_be_bytes(entry[12..20].try_into().unwrap());
        let size = u64::from_be_bytes(entry[20..28].try_into().unwrap());

        // Only initialize with memory-type region.
        if (pci_space >> 24) == 0x2 {
            MMIO_ALLOCATOR
                .call_once(|| SpinLock::new(MmioAllocator::new(cpu_addr as usize, size as usize)));
            break;
        }
    }
}

/// Allocates an MMIO address range using the global allocator.
pub(crate) fn alloc_mmio(layout: Layout) -> Option<Paddr> {
    MMIO_ALLOCATOR.get()?.lock().allocate(layout)
}

pub(crate) type MappedPciIrqLine = MappedIrqLine;

#[derive(Clone, Copy)]
struct PciIntxMapEntry {
    child_unit_address: u32,
    child_interrupt_pin: u32,
    interrupt_parent: u32,
    interrupt: u32,
}

struct PciIntxMapper {
    mask_unit_address: u32,
    mask_interrupt_pin: u32,
    entries: Vec<PciIntxMapEntry>,
}

fn init_intx_mapper_from_fdt(node: &FdtNode) {
    let Some(mask) = node.property("interrupt-map-mask") else {
        warn!("PCIe node has no `interrupt-map-mask` property");
        return;
    };
    let mask = mask.value;
    if mask.len() != 16 {
        warn!(
            "`interrupt-map-mask` should be 16 bytes, but found {} bytes",
            mask.len()
        );
        return;
    }

    let mask_unit_address = u32::from_be_bytes(mask[0..4].try_into().unwrap());
    let mask_interrupt_pin = u32::from_be_bytes(mask[12..16].try_into().unwrap());

    let Some(interrupt_map) = node.property("interrupt-map") else {
        warn!("PCIe node has no `interrupt-map` property");
        return;
    };

    let mut entries = Vec::new();
    // Each `interrupt-map` entry is laid out as:
    //   child unit address (3 cells) + child interrupt pin (1 cell)
    //   + interrupt parent phandle (1 cell) + parent interrupt specifier (1 cell)
    // = 6 cells = 24 bytes.
    //
    // This assumes the interrupt parent is a PLIC with `#interrupt-cells = 1`, which
    // matches ostd's `InterruptSourceInFdt` model (a single interrupt source number).
    // A controller with multi-cell specifiers (e.g. a GIC) is not supported.
    const ENTRY_SIZE: usize = 24;
    let remainder = interrupt_map.value.len() % ENTRY_SIZE;
    if remainder != 0 {
        warn!(
            "`interrupt-map` length is not a multiple of {ENTRY_SIZE}; {} trailing bytes dropped",
            remainder
        );
    }
    for entry in interrupt_map.value.chunks_exact(ENTRY_SIZE) {
        let child_unit_address = u32::from_be_bytes(entry[0..4].try_into().unwrap());
        let child_interrupt_pin = u32::from_be_bytes(entry[12..16].try_into().unwrap());
        let interrupt_parent = u32::from_be_bytes(entry[16..20].try_into().unwrap());
        let interrupt = u32::from_be_bytes(entry[20..24].try_into().unwrap());

        entries.push(PciIntxMapEntry {
            child_unit_address,
            child_interrupt_pin,
            interrupt_parent,
            interrupt,
        });
    }

    PCI_INTX_MAPPER.call_once(|| PciIntxMapper {
        mask_unit_address,
        mask_interrupt_pin,
        entries,
    });
}

pub(crate) fn map_intx_interrupt(location: &PciDeviceLocation) -> Result<MappedPciIrqLine, Error> {
    let interrupt_pin = location.read8(PciCommonCfgOffset::InterruptPin as u16) as u32;
    if interrupt_pin == 0 {
        return Err(Error::InvalidArgs);
    }

    let unit_address = ((location.device as u32) << 11) | ((location.function as u32) << 8);
    let mapper = PCI_INTX_MAPPER.get().ok_or(Error::InvalidArgs)?;
    let entry = mapper
        .entries
        .iter()
        .find(|entry| {
            (entry.child_unit_address & mapper.mask_unit_address)
                == (unit_address & mapper.mask_unit_address)
                && (entry.child_interrupt_pin & mapper.mask_interrupt_pin)
                    == (interrupt_pin & mapper.mask_interrupt_pin)
        })
        .ok_or(Error::InvalidArgs)?;

    let irq_line = IrqLine::alloc()?;
    IRQ_CHIP.get().unwrap().map_fdt_pin_to(
        InterruptSourceInFdt {
            interrupt_parent: entry.interrupt_parent,
            interrupt: entry.interrupt,
        },
        irq_line,
    )
}

/// The default target physical address written into each MSI-X Table entry's
/// `Message Address` field. A device raises an MSI by writing to this address.
///
/// On the QEMU `virt` platform this is `0x2400_0000`, the IMSIC (Incoming MSI
/// Controller) doorbell base defined by the RISC-V AIA. Per-vector remapping
/// ([`construct_remappable_msix_address`]) is not yet implemented, so every
/// vector currently shares this single address.
pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
