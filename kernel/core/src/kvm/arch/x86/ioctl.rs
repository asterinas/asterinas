// SPDX-License-Identifier: MPL-2.0

//! The x86 KVM ioctl definitions and register layouts.

use ostd::arch::vm::{VcpuDescTable, VcpuRegs, VcpuSegment, VcpuSregs};

use crate::{
    ioc,
    prelude::*,
    util::ioctl::{InData, NoData, OutData},
};

const KVM_INTERRUPT_BITMAP_WORDS: usize = 4;

// VM ioctls.
pub(super) type SetTssAddr = ioc!(KVM_SET_TSS_ADDR, 0xae, 0x47, NoData);

// VCPU ioctls.
pub(super) type GetRegs = ioc!(KVM_GET_REGS, 0xae, 0x81, OutData<KvmRegs>);
pub(super) type SetRegs = ioc!(KVM_SET_REGS, 0xae, 0x82, InData<KvmRegs>);
pub(super) type GetSregs = ioc!(KVM_GET_SREGS, 0xae, 0x83, OutData<KvmSregs>);
pub(super) type SetSregs = ioc!(KVM_SET_SREGS, 0xae, 0x84, InData<KvmSregs>);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct KvmRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl From<VcpuRegs> for KvmRegs {
    fn from(regs: VcpuRegs) -> Self {
        Self {
            rax: regs.rax as u64,
            rbx: regs.rbx as u64,
            rcx: regs.rcx as u64,
            rdx: regs.rdx as u64,
            rsi: regs.rsi as u64,
            rdi: regs.rdi as u64,
            rsp: regs.rsp as u64,
            rbp: regs.rbp as u64,
            r8: regs.r8 as u64,
            r9: regs.r9 as u64,
            r10: regs.r10 as u64,
            r11: regs.r11 as u64,
            r12: regs.r12 as u64,
            r13: regs.r13 as u64,
            r14: regs.r14 as u64,
            r15: regs.r15 as u64,
            rip: regs.rip as u64,
            rflags: regs.rflags as u64,
        }
    }
}

impl From<KvmRegs> for VcpuRegs {
    fn from(regs: KvmRegs) -> Self {
        Self {
            rax: regs.rax as usize,
            rbx: regs.rbx as usize,
            rcx: regs.rcx as usize,
            rdx: regs.rdx as usize,
            rsi: regs.rsi as usize,
            rdi: regs.rdi as usize,
            rbp: regs.rbp as usize,
            rsp: regs.rsp as usize,
            r8: regs.r8 as usize,
            r9: regs.r9 as usize,
            r10: regs.r10 as usize,
            r11: regs.r11 as usize,
            r12: regs.r12 as usize,
            r13: regs.r13 as usize,
            r14: regs.r14 as usize,
            r15: regs.r15 as usize,
            rip: regs.rip as usize,
            rflags: regs.rflags as usize,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct KvmSregs {
    pub cs: VcpuSegment,
    pub ds: VcpuSegment,
    pub es: VcpuSegment,
    pub fs: VcpuSegment,
    pub gs: VcpuSegment,
    pub ss: VcpuSegment,
    pub tr: VcpuSegment,
    pub ldt: VcpuSegment,
    pub gdt: VcpuDescTable,
    pub idt: VcpuDescTable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; KVM_INTERRUPT_BITMAP_WORDS],
}

impl From<VcpuSregs> for KvmSregs {
    fn from(sregs: VcpuSregs) -> Self {
        Self {
            cs: sregs.cs,
            ds: sregs.ds,
            es: sregs.es,
            fs: sregs.fs,
            gs: sregs.gs,
            ss: sregs.ss,
            tr: sregs.tr,
            ldt: sregs.ldt,
            gdt: sregs.gdt,
            idt: sregs.idt,
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            cr8: 0,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        }
    }
}

impl From<KvmSregs> for VcpuSregs {
    fn from(sregs: KvmSregs) -> Self {
        Self {
            cs: sregs.cs,
            ds: sregs.ds,
            es: sregs.es,
            fs: sregs.fs,
            gs: sregs.gs,
            ss: sregs.ss,
            tr: sregs.tr,
            ldt: sregs.ldt,
            gdt: sregs.gdt,
            idt: sregs.idt,
            cr0: sregs.cr0,
            cr2: sregs.cr2,
            cr3: sregs.cr3,
            cr4: sregs.cr4,
            efer: sregs.efer,
            apic_base: sregs.apic_base,
            interrupt_bitmap: sregs.interrupt_bitmap,
        }
    }
}
