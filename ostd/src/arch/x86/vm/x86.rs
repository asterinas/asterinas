#![allow(missing_docs)]

pub use x86::{dtables::DescriptorTablePointer, segmentation::SegmentSelector};
pub use x86_64::registers::control::{Cr0, Cr0Flags, Cr2, Cr3, Cr4, Cr4Flags};

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

pub fn es() -> SegmentSelector {
    x86::segmentation::es()
}

pub fn cs() -> SegmentSelector {
    x86::segmentation::cs()
}

pub fn ss() -> SegmentSelector {
    x86::segmentation::ss()
}

pub fn ds() -> SegmentSelector {
    x86::segmentation::ds()
}

pub fn fs() -> SegmentSelector {
    x86::segmentation::fs()
}

pub fn gs() -> SegmentSelector {
    x86::segmentation::gs()
}

pub fn tr() -> SegmentSelector {
    unsafe { x86::task::tr() }
}

pub fn read_cr2_raw() -> u64 {
    Cr2::read_raw()
}

pub fn write_cr2_raw(value: u64) {
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

pub fn sgdt(gdtp: &mut DescriptorTablePointer<u64>) {
    unsafe {
        x86::dtables::sgdt(gdtp);
    }
}

pub fn sidt(idtp: &mut DescriptorTablePointer<u64>) {
    unsafe {
        x86::dtables::sidt(idtp);
    }
}

pub fn get_tr_base(tr: SegmentSelector, gdtp: &DescriptorTablePointer<u64>) -> u64 {
    let gdt_base = gdtp.base.cast::<u8>() as usize;
    let index = (tr.bits() & !0x7) as usize;
    let desc_ptr = (gdt_base + index) as *const TssDescriptor64;
    let desc = unsafe { &*desc_ptr };

    u64::from(desc.base0)
        | (u64::from(desc.base1) << 16)
        | (u64::from(desc.base2) << 24)
        | (u64::from(desc.base3) << 32)
}
