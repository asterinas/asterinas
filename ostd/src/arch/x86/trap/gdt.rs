// SPDX-License-Identifier: MPL-2.0

//! Configure the Global Descriptor Table (GDT).

use core::{
    cell::UnsafeCell,
    mem::offset_of,
    sync::atomic::{AtomicU64, Ordering},
};

use x86_64::{
    PrivilegeLevel, VirtAddr,
    instructions::tables::{lgdt, load_tss},
    registers::segmentation::{CS, Segment},
    structures::{
        DescriptorTablePointer,
        gdt::{Descriptor, SegmentSelector},
        tss::TaskStateSegment,
    },
};

use crate::{
    const_assert,
    cpu::{
        CpuId,
        local::{CpuLocal, StaticCpuLocal},
    },
    cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::Vaddr,
};

/// Initializes and loads the GDT and TSS.
///
/// The caller should only call this method once in the boot context for each available processor.
/// This is not a safety requirement, however, because calling this method again will do nothing
/// more than load the GDT and TSS with the same contents.
///
/// # Safety
///
/// The caller must ensure that no preemption can occur during the method, otherwise we may
/// accidentally load a wrong GDT and TSS that actually belongs to another CPU.
pub(super) unsafe fn init_on_cpu() {
    // No races because the caller guarantees that no preemption can occur.
    let gdt = GDT.get_on_cpu(CpuId::current_racy());

    let tss_ptr = UnsafeCell::raw_get(LOCAL_TSS.as_ptr());

    // FIXME: The segment limit in the descriptor created by `tss_segment_unchecked` does not
    // include the I/O port bitmap.

    // SAFETY: As a CPU-local variable, the TSS lives for `'static`.
    let tss_desc = unsafe { Descriptor::tss_segment_unchecked(tss_ptr) };
    let (tss0, tss1) = match tss_desc {
        Descriptor::SystemSegment(tss0, tss1) => (tss0, tss1),
        _ => unreachable!(),
    };
    gdt.tss0.store(tss0, Ordering::Relaxed);
    gdt.tss1.store(tss1, Ordering::Relaxed);

    // The bundled Linux x86-64 vDSO reads selector 0x7b using LSL. Its
    // segment limit encodes the current CPU in bits 0..12 and the NUMA node
    // above it. NUMA is not supported yet, so the node is always zero. CPUs
    // outside the vDSO's 12-bit range cannot be encoded; the kernel does not
    // expose that vDSO when the system has more than 4096 CPUs.
    let cpu_id = u32::from(CpuId::current_racy());
    let cpunode_desc = if cpu_id <= 0xfff {
        // Present ring-3 read-only, expand-down data segment, byte-granular
        // limit, with the accessed bit set. This matches Linux's getcpu GDT.
        CPUNODE_DESCRIPTOR_FLAGS | u64::from(cpu_id)
    } else {
        0
    };
    gdt.cpunode.store(cpunode_desc, Ordering::Relaxed);

    // The kernel CS is considered a global invariant set by the boot GDT. This method is not
    // intended for switching to a new kernel CS.
    assert_eq!(CS::get_reg(), KERNEL_CS);

    // Load the new GDT.
    let gdtr = DescriptorTablePointer {
        limit: (size_of_val(gdt) - 1) as u16,
        base: VirtAddr::new(core::ptr::from_ref(gdt).addr() as u64),
    };
    // SAFETY: The GDT is valid to load because:
    //  - It lives for `'static`.
    //  - It contains correct entries at correct indexes: the kernel code/data segments, the user
    //    code/data segments, the TSS segment, and the CPU/node descriptor.
    //  - Specifically, the TSS segment points to the CPU-local TSS of the current CPU.
    unsafe { lgdt(&gdtr) };

    // Load the TSS.
    //
    // SAFETY: The selector points to the TSS descriptors in the GDT.
    unsafe { load_tss(TSS_SEL) };
}

/// Returns the current CPU's GDT base address.
pub(in crate::arch) fn gdt_base(_guard: &DisabledLocalIrqGuard) -> Vaddr {
    GDT.as_ptr() as Vaddr
}

/// Returns the GDT limit.
pub(in crate::arch) fn gdt_limit() -> u16 {
    (size_of::<Gdt>() - 1) as u16
}

/// Returns the current CPU's TSS base address.
pub(in crate::arch) fn tss_base(_guard: &DisabledLocalIrqGuard) -> Vaddr {
    UnsafeCell::raw_get(LOCAL_TSS.as_ptr()) as Vaddr
}

// The linker script makes sure that the `.cpu_local_tss` section is at the beginning of the area
// that stores CPU-local variables. This is important because `trap.S` and `syscall.S` will assume
// this and treat the beginning of the CPU-local area as a TSS for loading and saving the kernel
// stack!
//
// No other special initialization is required because the kernel stack information is stored in
// the TSS when we start the userspace program. However, to ensure the store's safety, we should
// wrap the TSS in an `UnsafeCell`. See `syscall.S` for details.
//
// SAFETY: This is properly handled in the linker script.
#[unsafe(link_section = ".cpu_local_tss")]
static LOCAL_TSS: StaticCpuLocal<UnsafeCell<TaskStateSegment>> = {
    let tss = UnsafeCell::new(TaskStateSegment::new());
    // SAFETY: The `.cpu_local_tss` section is part of the CPU-local area.
    unsafe { CpuLocal::__new_static(tss) }
};

#[repr(C)]
struct Gdt {
    // Immutable part: They are constant because they are either set to zero or all of the used bits
    // are set at compile time, including the ACCESSED bit. So they will not be modified later.
    _reserved: u64,
    kcode64: u64,
    kdata: u64,
    _kcode32: u64,
    _ucode32: u64,
    udata: u64,
    ucode64: u64,
    // Mutable part: They are initialized at runtime; the LTR instruction will set the BUSY bit.
    tss0: AtomicU64,
    tss1: AtomicU64,
    _reserved_after_tss: [u64; 6],
    cpunode: AtomicU64,
}

cpu_local! {
    static GDT: Gdt = Gdt {
        _reserved: 0,
        kcode64: KCODE64,
        kdata: KDATA,
        _kcode32: 0, // Not used.
        _ucode32: 0, // Not used.
        udata: UDATA,
        ucode64: UCODE64,
        tss0: AtomicU64::new(0), // To be set in `init_on_cpu`.
        tss1: AtomicU64::new(0), // To be set in `init_on_cpu`.
        _reserved_after_tss: [0; 6],
        cpunode: AtomicU64::new(0), // To be set in `init_on_cpu`.
    };
}

// ========= Descriptors =========

// Kernel code and data descriptors.
//
// These are the exact, unique values that satisfy the requirements of the `syscall` instruction.
// The Intel manual says: "It is the responsibility of OS software to ensure that the descriptors
// (in GDT or LDT) referenced by those selector values correspond to the fixed values loaded into
// the descriptor caches; the SYSCALL instruction does not ensure this correspondence."
pub(in crate::arch) const KCODE64: u64 = 0x00AF_9B00_0000_FFFF;
pub(in crate::arch) const KDATA: u64 = 0x00CF_9300_0000_FFFF;

// A 32-bit code descriptor that is used in the boot stage only. See `boot/bsp_boot.S`.
pub(in crate::arch) const KCODE32: u64 = 0x00CF_9B00_0000_FFFF;

// User code and data descriptors.
//
// These are the exact, unique values that satisfy the requirements of the `sysret` instruction.
// The Intel manual says: "It is the responsibility of OS software to ensure that the descriptors
// (in GDT or LDT) referenced by those selector values correspond to the fixed values loaded into
// the descriptor caches; the SYSRET instruction does not ensure this correspondence."
const UCODE64: u64 = 0x00AF_FB00_0000_FFFF;
const UDATA: u64 = 0x00CF_F300_0000_FFFF;

// A byte-granular ring-3 expand-down data segment. Its 20-bit limit is
// retrieved by the Linux vDSO's LSL instruction; bit 54 sets 32-bit mode.
const CPUNODE_DESCRIPTOR_FLAGS: u64 = 0x0040_F500_0000_0000;

// ========== Selectors ==========

pub(in crate::arch) const KERNEL_CS: SegmentSelector =
    SegmentSelector::new(1, PrivilegeLevel::Ring0);
const KERNEL_SS: SegmentSelector = SegmentSelector::new(2, PrivilegeLevel::Ring0);
const_assert!(KERNEL_CS.0 & OFFSET_MASK == offset_of!(Gdt, kcode64) as u16);
const_assert!(KERNEL_SS.0 & OFFSET_MASK == offset_of!(Gdt, kdata) as u16);

pub(super) const USER_CS: SegmentSelector = SegmentSelector::new(6, PrivilegeLevel::Ring3);
pub(super) const USER_SS: SegmentSelector = SegmentSelector::new(5, PrivilegeLevel::Ring3);
const_assert!(USER_CS.0 & OFFSET_MASK == offset_of!(Gdt, ucode64) as u16);
const_assert!(USER_SS.0 & OFFSET_MASK == offset_of!(Gdt, udata) as u16);

pub(in crate::arch) const TSS_SEL: SegmentSelector = SegmentSelector::new(7, PrivilegeLevel::Ring0);
const_assert!(TSS_SEL.0 & OFFSET_MASK == offset_of!(Gdt, tss0) as u16);
const_assert!(TSS_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, tss1) - size_of::<u64>()) as u16);

// The prebuilt Linux x86-64 vDSO uses the fixed selector 0x7b (entry 15, RPL 3).
const CPUNODE_SEL: SegmentSelector = SegmentSelector::new(15, PrivilegeLevel::Ring3);
const_assert!(CPUNODE_SEL.0 == 0x7b);
const_assert!(CPUNODE_SEL.0 & OFFSET_MASK == offset_of!(Gdt, cpunode) as u16);

pub(in crate::arch) const SYSRET_SEL: SegmentSelector =
    SegmentSelector::new(4, PrivilegeLevel::Ring3);
pub(in crate::arch) const SYSCALL_SEL: SegmentSelector =
    SegmentSelector::new(1, PrivilegeLevel::Ring0);
const_assert!(SYSRET_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, udata) - size_of::<u64>()) as u16);
const_assert!(
    SYSRET_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, ucode64) - size_of::<u64>() * 2) as u16
);
const_assert!(SYSCALL_SEL.0 & OFFSET_MASK == offset_of!(Gdt, kcode64) as u16);
const_assert!(SYSCALL_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, kdata) - size_of::<u64>()) as u16);

// Masking out the lower 3 bits of a selector yields the offset to the GDT entry from the GDT base.
const OFFSET_MASK: u16 = !7;
