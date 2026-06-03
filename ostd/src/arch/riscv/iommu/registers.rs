// SPDX-License-Identifier: MPL-2.0

//! MMIO register interface for the RISC-V IOMMU.

use core::ptr::NonNull;

use fdt::Fdt;
use spin::Once;
use volatile::{
    VolatileRef,
    access::{ReadOnly, ReadWrite},
};

use super::IommuError;
use crate::{
    arch::boot::DEVICE_TREE,
    io::IoMemAllocatorBuilder,
    sync::{LocalIrqDisabled, SpinLock},
};

const IOMMU_COMPATIBLE: &str = "riscv,iommu";

// Register offsets.
const REG_CAPABILITIES: usize = 0x00;
const REG_FCTL: usize = 0x08;
const REG_DDTP: usize = 0x10;
const REG_CQB: usize = 0x18;
const REG_CQH: usize = 0x20;
const REG_CQT: usize = 0x24;
const REG_FQB: usize = 0x28;
const REG_FQH: usize = 0x30;
const REG_FQT: usize = 0x34;
const REG_PQB: usize = 0x38;
const REG_PQH: usize = 0x40;
const REG_PQT: usize = 0x44;
const REG_CQCSR: usize = 0x48;
const REG_FQCSR: usize = 0x4C;
const REG_PQCSR: usize = 0x50;
const REG_IPSR: usize = 0x54;
const REG_ICVEC: usize = 0x2F8;

// Queue CSR bit definitions.
pub(super) const CQCSR_CQEN: u32 = 1 << 0;
pub(super) const CQCSR_CIE: u32 = 1 << 1;
pub(super) const CQCSR_CQMF: u32 = 1 << 8;
pub(super) const CQCSR_CMD_TO: u32 = 1 << 9;
pub(super) const CQCSR_CMD_ILL: u32 = 1 << 10;
pub(super) const CQCSR_FENCE_W_IP: u32 = 1 << 11;
pub(super) const CQCSR_CQON: u32 = 1 << 16;
pub(super) const CQCSR_BUSY: u32 = 1 << 17;

pub(super) const FQCSR_FQEN: u32 = 1 << 0;
pub(super) const FQCSR_FIE: u32 = 1 << 1;
pub(super) const FQCSR_FQMF: u32 = 1 << 8;
pub(super) const FQCSR_FQOF: u32 = 1 << 9;
pub(super) const FQCSR_FQON: u32 = 1 << 16;
pub(super) const FQCSR_BUSY: u32 = 1 << 17;

pub(super) const PQCSR_PQEN: u32 = 1 << 0;
pub(super) const PQCSR_PIE: u32 = 1 << 1;
pub(super) const PQCSR_PQMF: u32 = 1 << 8;
pub(super) const PQCSR_PQOF: u32 = 1 << 9;
pub(super) const PQCSR_PQON: u32 = 1 << 16;
pub(super) const PQCSR_BUSY: u32 = 1 << 17;

// ipsr bit definitions.
pub(super) const IPSR_CIP: u32 = 1 << 0;
pub(super) const IPSR_FIP: u32 = 1 << 1;
pub(super) const IPSR_PMIP: u32 = 1 << 2;
pub(super) const IPSR_PIP: u32 = 1 << 3;

// ddtp mode encodings.
pub(super) const DDTP_MODE_OFF: u64 = 0;
pub(super) const DDTP_MODE_BARE: u64 = 1;
pub(super) const DDTP_MODE_1LVL: u64 = 2;
pub(super) const DDTP_MODE_2LVL: u64 = 3;
pub(super) const DDTP_MODE_3LVL: u64 = 4;

/// Capability register (offset 0x00, 64-bit RO).
#[derive(Clone, Copy, Debug)]
pub struct Capability(u64);

impl Capability {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn version(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    pub const fn flags(&self) -> CapabilityFlags {
        CapabilityFlags::from_bits_truncate(self.0)
    }

    pub const fn physical_address_size(&self) -> u8 {
        ((self.0 >> 32) & 0x3F) as u8
    }
}

bitflags::bitflags! {
    pub struct CapabilityFlags: u64 {
        const SV32 = 1 << 8;
        const SV39 = 1 << 9;
        const SV48 = 1 << 10;
        const SV57 = 1 << 11;
        const SVRESV = 1 << 14;
        const SVPBMT = 1 << 15;
        const SV32X4 = 1 << 16;
        const SV39X4 = 1 << 17;
        const SV48X4 = 1 << 18;
        const SV57X4 = 1 << 19;
        const AMO_MRIF = 1 << 21;
        const MSI_FLAT = 1 << 22;
        const MSI_MRIF = 1 << 23;
        const AMO_HWAD = 1 << 24;
        const ATS = 1 << 25;
        const T2GPA = 1 << 26;
        const END = 1 << 27;
        const PD8 = 1 << 38;
        const PD17 = 1 << 39;
        const PD20 = 1 << 40;
        const QOSID = 1 << 41;
        const NL = 1 << 42;
        const S = 1 << 43;
    }
}

/// Features control register (offset 0x08, 32-bit WARL).
#[derive(Clone, Copy, Debug)]
pub struct FeaturesControl(u32);

impl FeaturesControl {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(&self) -> u32 {
        self.0
    }

    pub const fn be(&self) -> bool {
        (self.0 & 1) != 0
    }

    pub const fn wsi(&self) -> bool {
        (self.0 & (1 << 1)) != 0
    }

    pub const fn gxl(&self) -> bool {
        (self.0 & (1 << 2)) != 0
    }
}

/// Device directory table pointer register (offset 0x10, 64-bit WARL).
#[derive(Clone, Copy, Debug)]
pub struct Ddtp(u64);

impl Ddtp {
    pub fn new() -> Self {
        Self(0)
    }

    pub fn mode(&self) -> u8 {
        (self.0 & 0xF) as u8
    }

    pub fn set_mode(&mut self, mode: u8) {
        self.0 = (self.0 & !0xF) | (mode as u64 & 0xF);
    }

    pub fn ppn(&self) -> u64 {
        (self.0 >> 10) & 0xFFFFFFFFFFF
    }

    pub fn set_ppn(&mut self, ppn: u64) {
        self.0 = (self.0 & 0xFC00_0000_0000_03FF) | ((ppn & 0xFFFFFFFFFFF) << 10);
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Queue base register (used for cqb, fqb, pqb; 64-bit WARL).
#[derive(Clone, Copy, Debug)]
pub struct QueueBase(u64);

impl QueueBase {
    pub fn new(ppn: u64, log2sz_minus_1: u8) -> Self {
        Self(((ppn & 0xFFFFFFFFFFF) << 10) | (log2sz_minus_1 as u64 & 0x1F))
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

/// Queue Control/Status Register (cqcsr/fqcsr/pqcsr; 32-bit).
#[derive(Clone, Copy, Debug)]
pub struct QueueCsr(u32);

impl QueueCsr {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    pub fn value(&self) -> u32 {
        self.0
    }

    pub fn is_queued(&self) -> bool {
        (self.0 & 0x1) != 0
    }

    pub fn is_busy(&self) -> bool {
        (self.0 & (1 << 17)) != 0
    }
}

/// Volatile memory-mapped access to the IOMMU register block.
// TODO: Add `msi_cfg_tbl` (32 × 16 bytes at 0x300–0x3FF) for MSI interrupt
// vector configuration when interrupt remapping is implemented.
pub(super) struct IommuRegisters {
    pub capabilities: Capability,
    pub fctl: VolatileRef<'static, u32, ReadWrite>,
    pub ddtp: VolatileRef<'static, u64, ReadWrite>,
    pub cqb: VolatileRef<'static, u64, ReadWrite>,
    pub cqh: VolatileRef<'static, u32, ReadOnly>,
    pub cqt: VolatileRef<'static, u32, ReadWrite>,
    pub fqb: VolatileRef<'static, u64, ReadWrite>,
    pub fqh: VolatileRef<'static, u32, ReadWrite>,
    pub fqt: VolatileRef<'static, u32, ReadOnly>,
    pub cqcsr: VolatileRef<'static, u32, ReadWrite>,
    pub fqcsr: VolatileRef<'static, u32, ReadWrite>,
    pub ipsr: VolatileRef<'static, u32, ReadWrite>,
    pub icvec: VolatileRef<'static, u64, ReadWrite>,
}

impl IommuRegisters {
    // Creates a volatile register interface from the IOMMU MMIO base address.
    //
    // # Safety
    //
    // The caller must ensure that `base_register_vaddr` points to a valid IOMMU
    // register region and that it has exclusive ownership of the IOMMU registers.
    unsafe fn new(base_register_vaddr: NonNull<u8>) -> Self {
        // SAFETY: The safety is upheld by the caller.
        unsafe {
            let capabilities = Capability::new(
                VolatileRef::new_read_only(base_register_vaddr.add(REG_CAPABILITIES).cast::<u64>())
                    .as_ptr()
                    .read(),
            );

            Self {
                capabilities,
                fctl: VolatileRef::new(base_register_vaddr.add(REG_FCTL).cast::<u32>()),
                ddtp: VolatileRef::new(base_register_vaddr.add(REG_DDTP).cast::<u64>()),
                cqb: VolatileRef::new(base_register_vaddr.add(REG_CQB).cast::<u64>()),
                cqh: VolatileRef::new_read_only(base_register_vaddr.add(REG_CQH).cast::<u32>()),
                cqt: VolatileRef::new(base_register_vaddr.add(REG_CQT).cast::<u32>()),
                fqb: VolatileRef::new(base_register_vaddr.add(REG_FQB).cast::<u64>()),
                fqh: VolatileRef::new(base_register_vaddr.add(REG_FQH).cast::<u32>()),
                fqt: VolatileRef::new_read_only(base_register_vaddr.add(REG_FQT).cast::<u32>()),
                cqcsr: VolatileRef::new(base_register_vaddr.add(REG_CQCSR).cast::<u32>()),
                fqcsr: VolatileRef::new(base_register_vaddr.add(REG_FQCSR).cast::<u32>()),
                ipsr: VolatileRef::new(base_register_vaddr.add(REG_IPSR).cast::<u32>()),
                icvec: VolatileRef::new(base_register_vaddr.add(REG_ICVEC).cast::<u64>()),
            }
        }
    }
}

/// Scans the device tree for a node with `compatible = "riscv,iommu"` and
/// returns the base physical address from its `reg` property.
fn probe_iommu_from_fdt(fdt: &Fdt<'_>) -> Option<(usize, usize)> {
    fdt.all_nodes().find_map(|node| {
        if node
            .compatible()
            .is_some_and(|compatibles| compatibles.all().any(|c| c == IOMMU_COMPATIBLE))
        {
            let region = node.reg()?.next()?;
            let base = region.starting_address as usize;
            let size = region.size?;
            Some((base, size))
        } else {
            None
        }
    })
}

/// Global IOMMU register singleton.
pub(super) static IOMMU_REGS: Once<SpinLock<IommuRegisters, LocalIrqDisabled>> = Once::new();

/// Discovers and maps the IOMMU MMIO register region.
pub(super) fn init(io_mem_builder: &IoMemAllocatorBuilder) -> Result<(), IommuError> {
    let fdt = DEVICE_TREE.get().ok_or(IommuError::NoIommu)?;
    let (base_address, region_size) = probe_iommu_from_fdt(fdt).ok_or(IommuError::NoIommu)?;

    if base_address == 0 || region_size == 0 {
        return Err(IommuError::NoIommu);
    }

    let region_end = base_address
        .checked_add(region_size)
        .ok_or(IommuError::NoIommu)?;
    let range = base_address..region_end;
    io_mem_builder.remove(range);

    let base = NonNull::new(crate::mm::paddr_to_vaddr(base_address) as *mut u8)
        .ok_or(IommuError::NoIommu)?;

    // SAFETY: The safety is upheld by the device tree providing a valid IOMMU base
    // address and length, and `io_mem_builder.remove()` guaranteeing exclusive
    // ownership of the MMIO region.
    let iommu_regs = unsafe { IommuRegisters::new(base) };

    IOMMU_REGS.call_once(|| SpinLock::new(iommu_regs));
    Ok(())
}
