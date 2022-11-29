//! util for x86_64, it will rename to x86_64 when depend x86_64 isn't necessary
use core::arch::asm;

use x86_64::registers::{control::Cr4Flags, segmentation::Segment64, xcontrol::XCr0Flags};

#[inline(always)]
pub fn read_rsp() -> usize {
    let val: usize;
    unsafe {
        asm!("mov {}, rsp", out(reg) val);
    }
    val
}

#[inline(always)]
pub fn in8(port: u16) -> u8 {
    // ::x86_64::instructions::port::Port::read()
    let val: u8;
    unsafe {
        asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn in16(port: u16) -> u16 {
    let val: u16;
    unsafe {
        asm!("in ax, dx", out("ax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn in32(port: u16) -> u32 {
    let val: u32;
    unsafe {
        asm!("in eax, dx", out("eax") val, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    val
}

#[inline(always)]
pub fn out8(port: u16, val: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn out16(port: u16, val: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") val, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn out32(port: u16, val: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn disable_interrupts() {
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
}

#[inline(always)]
pub fn enable_interrupts_and_hlt() {
    unsafe {
        asm!("sti; hlt", options(nomem, nostack));
    }
}

#[inline]
pub fn enable_interrupts() {
    unsafe {
        asm!("sti", options(nomem, nostack));
    }
}

pub const RING0: u16 = 0;
pub const RING3: u16 = 3;

pub const RFLAGS_IF: usize = 1 << 9;

#[inline(always)]
pub fn get_msr(id: u32) -> usize {
    let (high, low): (u32, u32);
    unsafe {
        asm!("rdmsr", in("ecx") id, out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    ((high as usize) << 32) | (low as usize)
}

#[inline(always)]
pub fn set_msr(id: u32, val: usize) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    unsafe {
        asm!("wrmsr", in("ecx") id, in("eax") low, in("edx") high, options(nostack, preserves_flags));
    }
}

pub const EFER_MSR: u32 = 0xC000_0080;
pub const STAR_MSR: u32 = 0xC000_0081;
pub const LSTAR_MSR: u32 = 0xC000_0082;
pub const SFMASK_MSR: u32 = 0xC000_0084;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct DescriptorTablePointer {
    /// Size of the DT.
    pub limit: u16,
    /// Pointer to the memory region containing the DT.
    pub base: usize,
}

/// Load a GDT.
#[inline(always)]
pub fn lgdt(gdt: &DescriptorTablePointer) {
    unsafe {
        asm!("lgdt [{}]", in(reg) gdt, options(readonly, nostack, preserves_flags));
    }
}

/// Load an IDT.
#[inline(always)]
pub fn lidt(idt: &DescriptorTablePointer) {
    unsafe {
        asm!("lidt [{}]", in(reg) idt, options(readonly, nostack, preserves_flags));
    }
}

/// Load the task state register using the `ltr` instruction.
#[inline(always)]
pub fn load_tss(sel: u16) {
    unsafe {
        asm!("ltr {0:x}", in(reg) sel, options(nomem, nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn set_cs(sel: u16) {
    unsafe {
        asm!(
        "push {sel}",
        "lea {tmp}, [1f + rip]",
        "push {tmp}",
        "retfq",
        "1:",
        sel = in(reg) sel as usize,
        tmp = lateout(reg) _,
        options(preserves_flags),
        );
    }
}

#[inline(always)]
pub fn set_ss(sel: u16) {
    unsafe {
        asm!("mov ss, {0:x}", in(reg) sel, options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn get_cr3() -> usize {
    let val: usize;
    unsafe {
        asm!("mov {}, cr3", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    // Mask top bits and flags.
    val & 0x_000f_ffff_ffff_f000
}

#[inline(always)]
pub fn get_cr3_raw() -> usize {
    let val: usize;
    unsafe {
        asm!("mov {}, cr3", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    // Mask top bits and flags.
    val
}

#[inline(always)]
pub fn get_return_address() -> usize {
    let val: usize;
    unsafe {
        asm!("mov {}, [rsp]", out(reg) val);
    }
    val
}

#[inline(always)]
pub fn set_cr3(pa: usize) {
    unsafe {
        asm!("mov cr3, {}", in(reg) pa, options(nostack, preserves_flags));
    }
}

#[inline(always)]
pub fn wrfsbase(base: u64) {
    unsafe { asm!("wrfsbase {0}", in(reg) base, options(att_syntax)) }
}

#[inline(always)]
pub fn rdfsbase() -> u64 {
    let fs_base = x86_64::registers::segmentation::FS::read_base();
    fs_base.as_u64()
}

pub fn enable_common_cpu_features() {
    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::FSGSBASE | Cr4Flags::OSXSAVE | Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE;
    unsafe {
        x86_64::registers::control::Cr4::write(cr4);
    }

    let mut xcr0 = x86_64::registers::xcontrol::XCr0::read();
    xcr0 |= XCr0Flags::AVX | XCr0Flags::SSE;
    unsafe {
        x86_64::registers::xcontrol::XCr0::write(xcr0);
    }
}

pub fn flush_tlb() {
    x86_64::instructions::tlb::flush_all();
}
