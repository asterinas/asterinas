// SPDX-License-Identifier: MPL-2.0

//! Configure the Global Descriptor Table (GDT).

use alloc::boxed::Box;

use x86_64::{
    instructions::tables::{lgdt, load_tss},
    registers::{
        model_specific::Star,
        segmentation::{Segment, CS},
    },
    structures::{
        gdt::{Descriptor, SegmentSelector},
        tss::TaskStateSegment,
        DescriptorTablePointer,
    },
    PrivilegeLevel, VirtAddr,
};

use crate::cpu::local::{CpuLocal, StaticCpuLocal};

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
pub(super) unsafe fn init() {
    let tss_ptr = LOCAL_TSS.as_ptr();

    // FIXME: The segment limit in the descriptor created by `tss_segment_unchecked` does not
    // include the I/O port bitmap.

    // SAFETY: As a CPU-local variable, the TSS lives for `'static`.
    let tss_desc = unsafe { Descriptor::tss_segment_unchecked(tss_ptr) };
    let (tss0, tss1) = match tss_desc {
        Descriptor::SystemSegment(tss0, tss1) => (tss0, tss1),
        _ => unreachable!(),
    };

    // The kernel CS is considered a global invariant set by the boot GDT. This method is not
    // intended for switching to a new kernel CS.
    assert_eq!(CS::get_reg(), KERNEL_CS);

    // Allocate a new GDT with 8 entries.
    let gdt = Box::new([
        0, KCODE64, KDATA, /* UCODE32 (not used) */ 0, UDATA, UCODE64, tss0, tss1,
    ]);
    let gdt = &*Box::leak(gdt);
    assert_eq!(gdt[KERNEL_CS.index() as usize], KCODE64);
    assert_eq!(gdt[KERNEL_SS.index() as usize], KDATA);
    assert_eq!(gdt[USER_CS.index() as usize], UCODE64);
    assert_eq!(gdt[USER_SS.index() as usize], UDATA);

    // Load the new GDT.
    let gdtr = DescriptorTablePointer {
        limit: (core::mem::size_of_val(gdt) - 1) as u16,
        base: VirtAddr::new(gdt.as_ptr().addr() as u64),
    };
    // SAFETY: The GDT is valid to load because:
    //  - It lives for `'static`.
    //  - It contains correct entries at correct indexes: the kernel code/data segments, the user
    //    code/data segments, and the TSS segment.
    //  - Specifically, the TSS segment points to the CPU-local TSS of the current CPU.
    unsafe { lgdt(&gdtr) };

    // Load the TSS.
    let tss_sel = SegmentSelector::new(6, PrivilegeLevel::Ring0);
    assert_eq!(gdt[tss_sel.index() as usize], tss0);
    assert_eq!(gdt[(tss_sel.index() + 1) as usize], tss1);
    // SAFETY: The selector points to the TSS descriptors in the GDT.
    unsafe { load_tss(tss_sel) };

    // Set up the selectors for the `syscall` and `sysret` instructions.
    let sysret = SegmentSelector::new(3, PrivilegeLevel::Ring3);
    assert_eq!(gdt[(sysret.index() + 1) as usize], UDATA);
    assert_eq!(gdt[(sysret.index() + 2) as usize], UCODE64);
    let syscall = SegmentSelector::new(1, PrivilegeLevel::Ring0);
    assert_eq!(gdt[syscall.index() as usize], KCODE64);
    assert_eq!(gdt[(syscall.index() + 1) as usize], KDATA);
    // SAFETY: The selector points to correct kernel/user code/data descriptors in the GDT.
    unsafe { Star::write_raw(sysret.0, syscall.0) };
}

// The linker script makes sure that the `.cpu_local_tss` section is at the beginning of the area
// that stores CPU-local variables. This is important because `trap.S` and `syscall.S` will assume
// this and treat the beginning of the CPU-local area as a TSS for loading and saving the kernel
// stack!
//
// No other special initialization is required because the kernel stack information is stored in
// the TSS when we start the userspace program. See `syscall.S` for details.
#[link_section = ".cpu_local_tss"]
static LOCAL_TSS: StaticCpuLocal<TaskStateSegment> = {
    let tss = TaskStateSegment::new();
    // SAFETY: The `.cpu_local_tss` section is part of the CPU-local area.
    unsafe { CpuLocal::__new_static(tss) }
};

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

const KERNEL_CS: SegmentSelector = SegmentSelector::new(1, PrivilegeLevel::Ring0);
const KERNEL_SS: SegmentSelector = SegmentSelector::new(2, PrivilegeLevel::Ring0);

pub(super) const USER_CS: SegmentSelector = SegmentSelector::new(5, PrivilegeLevel::Ring3);
pub(super) const USER_SS: SegmentSelector = SegmentSelector::new(4, PrivilegeLevel::Ring3);
