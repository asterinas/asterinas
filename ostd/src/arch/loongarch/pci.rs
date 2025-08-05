// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use core::alloc::Layout;

use align_ext::AlignExt;
use fdt::node::FdtNode;
use log::warn;
use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{
    bus::pci::PciDeviceLocation, io::IoMem, mm::VmIoOnce, prelude::*, sync::SpinLock, Error,
};

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | (offset & 0xfc)) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | (offset & 0xfc)) as usize)
}

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    // We only support ECAM here for LoongArch platforms. Offsets are from
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    ((location.bus as u32) << 20)
        | ((location.device as u32) << 15)
        | ((location.function as u32) << 12)
}

pub(crate) fn has_pci_bus() -> bool {
    PCI_ECAM_CFG_SPACE.is_completed()
}

pub(crate) fn init() {
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
        warn!("No generic PCI host controller node found in the device tree");
        return;
    };

    let Some(mut reg) = pci.reg() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return;
    };
    let Some(region) = reg.next() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return;
    };
    if reg.next().is_some() {
        warn!(
            "PCI node should have exactly one `reg` property, but found {} `reg`s",
            reg.count() + 2
        );
        return;
    }

    // Initialize the MMIO allocator
    init_mmio_allocator_from_fdt(&pci);

    let addr_start = region.starting_address as usize;
    let addr_end = addr_start.checked_add(region.size.unwrap()).unwrap();
    PCI_ECAM_CFG_SPACE.call_once(|| IoMem::acquire(addr_start..addr_end).unwrap());
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2ff0_0000;

pub(crate) fn construct_remappable_msix_address(remapping_index: u32) -> u32 {
    unimplemented!()
}

/// A simple MMIO allocator managing a linear region.
///
/// In loongarch, the starting address of the memory bar of the PCI device
/// needs to be allocated within the specified range
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
        let align = layout.align();
        let size = layout.size();

        let current = self.base + self.offset;
        let aligned = current.align_up(align);
        let aligned_offset = aligned - self.base;

        if aligned_offset + size > self.size {
            return None;
        }
        self.offset = aligned_offset + size;
        Some(aligned)
    }
}

static MMIO_ALLOCATOR: Once<SpinLock<MmioAllocator>> = Once::new();

/// Initializes the MMIO allocator from the PCIe node's "ranges" property.
fn init_mmio_allocator_from_fdt(node: &FdtNode) {
    let ranges = node
        .property("ranges")
        .expect("Missing 'ranges' property in PCIe node");
    let data = ranges.value;

    let entry_size = 7 * 4; // Each entry is 7 x u32 = 28 bytes
    let mut i = 0;

    while i + entry_size <= data.len() {
        let pci_space = u32::from_be_bytes(data[i..i + 4].try_into().unwrap());
        let pci_addr = u64::from_be_bytes(data[i + 4..i + 12].try_into().unwrap());
        let cpu_addr = u64::from_be_bytes(data[i + 12..i + 20].try_into().unwrap());
        let size = u64::from_be_bytes(data[i + 20..i + 28].try_into().unwrap());

        // Only initialize with memory-type region
        if (pci_space >> 24) == 0x2 {
            MMIO_ALLOCATOR
                .call_once(|| SpinLock::new(MmioAllocator::new(cpu_addr as usize, size as usize)));
            break;
        }

        i += entry_size;
    }
}

/// Allocates an MMIO address range using the global allocator.
pub(crate) fn alloc_mmio(layout: Layout) -> Option<Paddr> {
    MMIO_ALLOCATOR.get().unwrap().lock().allocate(layout)
}
