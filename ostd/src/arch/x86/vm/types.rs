// SPDX-License-Identifier: MPL-2.0

//! Types that represent guest-visible x86 CPU state.

/// Guest general-purpose registers, instruction pointer, and flags.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuRegs {
    /// The `RAX` register.
    pub rax: usize,
    /// The `RBX` register.
    pub rbx: usize,
    /// The `RCX` register.
    pub rcx: usize,
    /// The `RDX` register.
    pub rdx: usize,
    /// The `RSI` register.
    pub rsi: usize,
    /// The `RDI` register.
    pub rdi: usize,
    /// The `RBP` register.
    pub rbp: usize,
    /// The stack pointer.
    pub rsp: usize,
    /// The `R8` register.
    pub r8: usize,
    /// The `R9` register.
    pub r9: usize,
    /// The `R10` register.
    pub r10: usize,
    /// The `R11` register.
    pub r11: usize,
    /// The `R12` register.
    pub r12: usize,
    /// The `R13` register.
    pub r13: usize,
    /// The `R14` register.
    pub r14: usize,
    /// The `R15` register.
    pub r15: usize,
    /// The instruction pointer.
    pub rip: usize,
    /// The flags register.
    pub rflags: usize,
}

/// Guest special-register state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSregs {
    /// The code segment.
    pub cs: VcpuSegment,
    /// The data segment.
    pub ds: VcpuSegment,
    /// The extra data segment.
    pub es: VcpuSegment,
    /// The `FS` segment, including the `IA32_FS_BASE` MSR value.
    pub fs: VcpuSegment,
    /// The `GS` segment, including the `IA32_GS_BASE` MSR value.
    pub gs: VcpuSegment,
    /// The stack segment.
    pub ss: VcpuSegment,
    /// The task register and its cached segment descriptor.
    pub tr: VcpuSegment,
    /// The local descriptor-table register and its cached segment descriptor.
    pub ldt: VcpuSegment,
    /// The global descriptor-table register.
    pub gdt: VcpuDescTable,
    /// The interrupt descriptor-table register.
    pub idt: VcpuDescTable,
    /// The `CR0` control register.
    pub cr0: u64,
    /// The page-fault linear address.
    pub cr2: u64,
    /// The page-table root and control bits.
    pub cr3: u64,
    /// The `CR4` control register.
    pub cr4: u64,
    /// The extended feature enable register.
    pub efer: u64,
    /// The local APIC base address and control bits.
    pub apic_base: u64,
    /// The pending interrupt vectors, with one bit per vector.
    pub interrupt_bitmap: [u64; 4],
}

/// Guest segment-register state.
///
/// Refer to: Intel SDM Vol. 3A, 3.4.5 "Segment Descriptors".
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuSegment {
    /// The segment base address.
    pub base: u64,
    /// The effective segment limit in bytes, with granularity already applied.
    pub limit: u32,
    /// The segment selector.
    pub selector: u16,
    /// The four-bit segment type.
    pub type_: u8,
    /// The segment-present bit.
    pub present: u8,
    /// The descriptor privilege level.
    pub dpl: u8,
    /// The default operation size or upper-bound bit.
    pub db: u8,
    /// The descriptor class bit: zero for system, one for code or data.
    pub s: u8,
    /// The 64-bit code-segment bit.
    pub l: u8,
    /// The granularity bit: zero for byte, one for 4-KiB granularity.
    pub g: u8,
    /// The descriptor bit available to software.
    pub avl: u8,
    /// The VMX unusable-segment bit.
    pub unusable: u8,
    /// The reserved padding byte.
    pub padding: u8,
}

/// Guest descriptor-table state.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuDescTable {
    /// The linear base address of the descriptor table.
    pub base: u64,
    /// The table size in bytes minus one.
    pub limit: u16,
    /// The reserved padding words.
    pub padding: [u16; 3],
}

/// Guest MSR state that is not already stored in [`VcpuSregs`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VcpuMsrs {
    /// The page attribute table.
    pub pat: u64,
    /// The kernel GS base exchanged by `SWAPGS`.
    pub kernel_gs_base: u64,
    /// The `SYSCALL` and `SYSRET` segment selectors.
    pub star: u64,
    /// The 64-bit `SYSCALL` entry point.
    pub lstar: u64,
    /// The compatibility-mode `SYSCALL` entry point.
    pub cstar: u64,
    /// The flags cleared on `SYSCALL` entry.
    pub syscall_mask: u64,
    /// The accumulated TSC adjustment.
    pub tsc_adjust: u64,
    /// The auxiliary TSC value returned by `RDTSCP`.
    pub tsc_aux: u64,
    /// The `SYSENTER` code-segment selector.
    pub sysenter_cs: u64,
    /// The `SYSENTER` stack pointer.
    pub sysenter_esp: u64,
    /// The `SYSENTER` instruction pointer.
    pub sysenter_eip: u64,
    /// The miscellaneous processor feature controls.
    pub misc_enable: u64,
}
