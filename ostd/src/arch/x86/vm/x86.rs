#![allow(missing_docs)]

use x86::{dtables::DescriptorTablePointer, segmentation::SegmentSelector};

#[repr(C)]
struct TssDescriptor64 {
    limit0: u16,
    base0: u16,
    base1: u8,
    flags1: u8,
    flags2: u8,
    base2: u8,
    base3: u32,
    reserved: u32,
}

pub(crate) fn write_cr2_raw(value: u64) {
    // SAFETY: CR2 is the page-fault linear-address register. Updating it does
    // not change paging, privilege, or memory mappings; RustShyper uses this to
    // swap host and guest CR2 values around VM entry/exit.
    unsafe {
        core::arch::asm!(
            "mov cr2, {}",
            in(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub(super) fn get_tr_base(tr: SegmentSelector, gdtp: &DescriptorTablePointer<u64>) -> u64 {
    let gdt_base = gdtp.base.cast::<u8>() as usize;
    let index = (tr.bits() & !0x7) as usize;
    let desc_ptr = (gdt_base + index) as *const TssDescriptor64;
    let desc = unsafe { &*desc_ptr };

    u64::from(desc.base0)
        | (u64::from(desc.base1) << 16)
        | (u64::from(desc.base2) << 24)
        | (u64::from(desc.base3) << 32)
}
