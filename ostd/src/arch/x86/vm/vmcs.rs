// SPDX-License-Identifier: MPL-2.0

//! VMCS execution controls and host/guest state synchronization.

use x86::{
    dtables::{self, DescriptorTablePointer},
    msr::*,
    segmentation, task,
    vmx::vmcs::{
        control::{
            self, EntryControls, ExitControls, PinbasedControls, PrimaryControls, SecondaryControls,
        },
        guest, host,
    },
};
use x86_64::registers::{
    control::{Cr0, Cr0Flags, Cr4},
    model_specific::EferFlags,
};

use super::{
    context::VcpuArchState,
    types::VcpuSegment,
    vmx::{VmxGuard, context_switch, vmcs::Vmcs},
};
use crate::{Error, cpu::PinCurrentCpu, irq::DisabledLocalIrqGuard, prelude::*};

impl Vmcs {
    pub(super) fn setup_controls(
        &self,
        eptp: u64,
        guest_efer: u64,
        _vmx_guard: &VmxGuard,
        _pin_guard: &impl PinCurrentCpu,
    ) -> Result<()> {
        let basic = unsafe { rdmsr(IA32_VMX_BASIC) };
        let has_true_controls = basic & (1 << 55) != 0;
        self.set_control(
            control::PINBASED_EXEC_CONTROLS,
            if has_true_controls {
                IA32_VMX_TRUE_PINBASED_CTLS
            } else {
                IA32_VMX_PINBASED_CTLS
            },
            (PinbasedControls::EXTERNAL_INTERRUPT_EXITING
                | PinbasedControls::NMI_EXITING
                | PinbasedControls::VMX_PREEMPTION_TIMER)
                .bits(),
            0,
        )?;
        self.set_control(
            control::PRIMARY_PROCBASED_EXEC_CONTROLS,
            if has_true_controls {
                IA32_VMX_TRUE_PROCBASED_CTLS
            } else {
                IA32_VMX_PROCBASED_CTLS
            },
            (PrimaryControls::HLT_EXITING
                | PrimaryControls::UNCOND_IO_EXITING
                | PrimaryControls::SECONDARY_CONTROLS)
                .bits(),
            (PrimaryControls::CR3_LOAD_EXITING | PrimaryControls::CR3_STORE_EXITING).bits(),
        )?;
        self.set_control(
            control::SECONDARY_PROCBASED_EXEC_CONTROLS,
            IA32_VMX_PROCBASED_CTLS2,
            (SecondaryControls::ENABLE_EPT | SecondaryControls::UNRESTRICTED_GUEST).bits(),
            0,
        )?;
        self.set_control(
            control::VMEXIT_CONTROLS,
            if has_true_controls {
                IA32_VMX_TRUE_EXIT_CTLS
            } else {
                IA32_VMX_EXIT_CTLS
            },
            (ExitControls::HOST_ADDRESS_SPACE_SIZE
                | ExitControls::SAVE_IA32_PAT
                | ExitControls::LOAD_IA32_PAT
                | ExitControls::SAVE_IA32_EFER
                | ExitControls::LOAD_IA32_EFER)
                .bits(),
            0,
        )?;
        let mut entry = EntryControls::LOAD_IA32_PAT | EntryControls::LOAD_IA32_EFER;
        if guest_efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0 {
            entry |= EntryControls::IA32E_MODE_GUEST;
        }
        self.set_control(
            control::VMENTRY_CONTROLS,
            if has_true_controls {
                IA32_VMX_TRUE_ENTRY_CTLS
            } else {
                IA32_VMX_ENTRY_CTLS
            },
            entry.bits(),
            0,
        )?;

        self.write(control::EXCEPTION_BITMAP, 0)?;
        self.write(control::PAGE_FAULT_ERR_CODE_MASK, 0)?;
        self.write(control::PAGE_FAULT_ERR_CODE_MATCH, 0)?;
        self.write(control::CR3_TARGET_COUNT, 0)?;
        self.write(control::VMEXIT_MSR_STORE_COUNT, 0)?;
        self.write(control::VMEXIT_MSR_LOAD_COUNT, 0)?;
        self.write(control::VMENTRY_MSR_LOAD_COUNT, 0)?;
        self.write(control::EPTP_FULL, eptp as usize)
    }

    fn set_control(
        &self,
        field: u32,
        capability_msr: u32,
        required: u32,
        forbidden: u32,
    ) -> Result<()> {
        // Intel SDM, Vol. 3D, Appendix A:
        // low capability bits are required ones, high bits are allowed ones.
        let capability = unsafe { rdmsr(capability_msr) };
        let value = (required | capability as u32) & (capability >> 32) as u32;
        if value & required != required || value & forbidden != 0 {
            return Err(Error::NotEnoughResources);
        }
        self.write(field, value as usize)
    }

    pub(super) fn setup_host(&self, _irq_guard: &DisabledLocalIrqGuard) -> Result<()> {
        // Refresh all host fields on every run, including the current task's
        // CR3 and FS base. A context may migrate or be run by a different task.
        self.write(host::CR0, Cr0::read_raw() as usize)?;
        // Preserve the PCID as well as the root address.
        let host_cr3: usize;
        // SAFETY: Reading CR3 does not change address translation.
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) host_cr3, options(nomem, nostack, preserves_flags));
        }
        self.write(host::CR3, host_cr3)?;
        self.write(host::CR4, Cr4::read_raw() as usize)?;
        self.write(host::ES_SELECTOR, segmentation::es().bits() as usize)?;
        self.write(host::CS_SELECTOR, segmentation::cs().bits() as usize)?;
        self.write(host::SS_SELECTOR, segmentation::ss().bits() as usize)?;
        self.write(host::DS_SELECTOR, segmentation::ds().bits() as usize)?;
        self.write(host::FS_SELECTOR, segmentation::fs().bits() as usize)?;
        self.write(host::GS_SELECTOR, segmentation::gs().bits() as usize)?;
        // SAFETY: These architectural MSRs are present on VMX-capable x86-64.
        // Reads only snapshot host state while the IRQ guard pins this CPU.
        unsafe {
            self.write(host::FS_BASE, rdmsr(IA32_FS_BASE) as usize)?;
            self.write(host::GS_BASE, rdmsr(IA32_GS_BASE) as usize)?;
            self.write(host::IA32_PAT_FULL, rdmsr(IA32_PAT) as usize)?;
            self.write(host::IA32_EFER_FULL, rdmsr(IA32_EFER) as usize)?;
            self.write(host::IA32_SYSENTER_CS, rdmsr(IA32_SYSENTER_CS) as usize)?;
            self.write(host::IA32_SYSENTER_ESP, rdmsr(IA32_SYSENTER_ESP) as usize)?;
            self.write(host::IA32_SYSENTER_EIP, rdmsr(IA32_SYSENTER_EIP) as usize)?;
        }

        let mut gdt = DescriptorTablePointer::<u64>::default();
        let mut idt = DescriptorTablePointer::<u64>::default();
        // SAFETY: STR, SGDT and SIDT only read the current host's descriptor
        // registers. The IRQ guard keeps the CPU and its tables unchanged.
        let tr = unsafe {
            dtables::sgdt(&mut gdt);
            dtables::sidt(&mut idt);
            task::tr()
        };
        self.write(host::TR_SELECTOR, tr.bits() as usize)?;
        // SAFETY: These are the current host's live GDT and TSS; the IRQ guard
        // prevents migration or replacement of this CPU's descriptor tables.
        self.write(host::TR_BASE, unsafe { super::x86::get_tr_base(tr, &gdt) }
            as usize)?;
        self.write(host::GDTR_BASE, gdt.base as usize)?;
        self.write(host::IDTR_BASE, idt.base as usize)?;
        self.write(host::RIP, context_switch::vm_exit_handler_virtaddr())
    }

    pub(super) fn load_guest_context(
        &self,
        arch: &VcpuArchState,
        _vmx: &VmxGuard,
        _irq_guard: &DisabledLocalIrqGuard,
    ) -> Result<()> {
        let sregs = &arch.sregs;

        let (cr0_fixed0, cr0_fixed1, cr4_fixed0, cr4_fixed1) = unsafe {
            (
                rdmsr(IA32_VMX_CR0_FIXED0),
                rdmsr(IA32_VMX_CR0_FIXED1),
                rdmsr(IA32_VMX_CR4_FIXED0),
                rdmsr(IA32_VMX_CR4_FIXED1),
            )
        };
        // Unrestricted guests need not have PE or PG set. Keep the architectural
        // values in the read shadows, including bits forced by VMX requirements.
        let cr0_fixed0 = cr0_fixed0 & !(Cr0Flags::PROTECTED_MODE_ENABLE | Cr0Flags::PAGING).bits();
        self.write(guest::CR0, ((sregs.cr0 | cr0_fixed0) & cr0_fixed1) as usize)?;
        self.write(guest::CR4, ((sregs.cr4 | cr4_fixed0) & cr4_fixed1) as usize)?;
        self.write(control::CR0_GUEST_HOST_MASK, usize::MAX)?;
        self.write(control::CR4_GUEST_HOST_MASK, usize::MAX)?;
        self.write(control::CR0_READ_SHADOW, sregs.cr0 as usize)?;
        self.write(control::CR4_READ_SHADOW, sregs.cr4 as usize)?;
        self.write(guest::CR3, sregs.cr3 as usize)?;

        for (segment, fields) in [
            (&sregs.cs, CS),
            (&sregs.ds, DS),
            (&sregs.es, ES),
            (&sregs.fs, FS),
            (&sregs.gs, GS),
            (&sregs.ss, SS),
            (&sregs.tr, TR),
            (&sregs.ldt, LDTR),
        ] {
            self.write_segment(segment, fields)?;
        }
        self.write(guest::GDTR_BASE, sregs.gdt.base as usize)?;
        self.write(guest::GDTR_LIMIT, sregs.gdt.limit as usize)?;
        self.write(guest::IDTR_BASE, sregs.idt.base as usize)?;
        self.write(guest::IDTR_LIMIT, sregs.idt.limit as usize)?;
        self.write(guest::RIP, arch.regs.rip)?;
        self.write(guest::RSP, arch.regs.rsp)?;
        self.write(guest::RFLAGS, arch.regs.rflags | 2)?;
        self.write(guest::IA32_EFER_FULL, sregs.efer as usize)?;
        self.write(guest::IA32_PAT_FULL, arch.msrs.pat as usize)?;
        self.write(guest::IA32_SYSENTER_CS, arch.msrs.sysenter_cs as usize)?;
        self.write(guest::IA32_SYSENTER_ESP, arch.msrs.sysenter_esp as usize)?;
        self.write(guest::IA32_SYSENTER_EIP, arch.msrs.sysenter_eip as usize)?;
        self.write(guest::INTERRUPTIBILITY_STATE, arch.interruptibility)?;
        // HLT exits are handled in software, so every explicit entry is active.
        self.write(guest::ACTIVITY_STATE, 0)?;
        self.write(guest::PENDING_DBG_EXCEPTIONS, 0)?;
        self.write(guest::LINK_PTR_FULL, usize::MAX)
    }

    pub(super) fn save_guest_context(&self, arch: &mut VcpuArchState) -> Result<()> {
        // Read into a temporary snapshot so a failed VMREAD does not publish a
        // mixture of pre-entry and post-exit special registers.
        let mut sregs = arch.sregs;
        sregs.cs = self.read_segment(CS)?;
        sregs.ds = self.read_segment(DS)?;
        sregs.es = self.read_segment(ES)?;
        sregs.fs = self.read_segment(FS)?;
        sregs.gs = self.read_segment(GS)?;
        sregs.ss = self.read_segment(SS)?;
        sregs.tr = self.read_segment(TR)?;
        sregs.ldt = self.read_segment(LDTR)?;
        sregs.gdt.base = self.read(guest::GDTR_BASE)? as u64;
        sregs.gdt.limit = self.read(guest::GDTR_LIMIT)? as u16;
        sregs.idt.base = self.read(guest::IDTR_BASE)? as u64;
        sregs.idt.limit = self.read(guest::IDTR_LIMIT)? as u16;
        sregs.cr3 = self.read(guest::CR3)? as u64;
        let rip = self.read(guest::RIP)?;
        let rsp = self.read(guest::RSP)?;
        let rflags = self.read(guest::RFLAGS)?;
        let interruptibility = self.read(guest::INTERRUPTIBILITY_STATE)?;

        arch.sregs = sregs;
        arch.regs.rip = rip;
        arch.regs.rsp = rsp;
        arch.regs.rflags = rflags;
        arch.interruptibility = interruptibility;
        Ok(())
    }

    fn write_segment(
        &self,
        segment: &VcpuSegment,
        [selector, base, limit, rights]: [u32; 4],
    ) -> Result<()> {
        let mut access = usize::from(segment.type_ & 0xf);
        access |= usize::from(segment.s & 1) << 4;
        access |= usize::from(segment.dpl & 3) << 5;
        access |= usize::from(segment.present & 1) << 7;
        access |= usize::from(segment.avl & 1) << 12;
        access |= usize::from(segment.l & 1) << 13;
        access |= usize::from(segment.db & 1) << 14;
        access |= usize::from(segment.g & 1) << 15;
        access |= usize::from(segment.unusable & 1) << 16;
        self.write(selector, segment.selector as usize)?;
        self.write(base, segment.base as usize)?;
        self.write(limit, segment.limit as usize)?;
        self.write(rights, access)
    }

    fn read_segment(&self, [selector, base, limit, rights]: [u32; 4]) -> Result<VcpuSegment> {
        let access = self.read(rights)?;
        Ok(VcpuSegment {
            base: self.read(base)? as u64,
            limit: self.read(limit)? as u32,
            selector: self.read(selector)? as u16,
            type_: (access & 0xf) as u8,
            present: ((access >> 7) & 1) as u8,
            dpl: ((access >> 5) & 3) as u8,
            db: ((access >> 14) & 1) as u8,
            s: ((access >> 4) & 1) as u8,
            l: ((access >> 13) & 1) as u8,
            g: ((access >> 15) & 1) as u8,
            avl: ((access >> 12) & 1) as u8,
            unusable: ((access >> 16) & 1) as u8,
            padding: 0,
        })
    }
}

// Each tuple names selector, base, limit and access-rights fields.
const CS: [u32; 4] = [
    guest::CS_SELECTOR,
    guest::CS_BASE,
    guest::CS_LIMIT,
    guest::CS_ACCESS_RIGHTS,
];
const DS: [u32; 4] = [
    guest::DS_SELECTOR,
    guest::DS_BASE,
    guest::DS_LIMIT,
    guest::DS_ACCESS_RIGHTS,
];
const ES: [u32; 4] = [
    guest::ES_SELECTOR,
    guest::ES_BASE,
    guest::ES_LIMIT,
    guest::ES_ACCESS_RIGHTS,
];
const FS: [u32; 4] = [
    guest::FS_SELECTOR,
    guest::FS_BASE,
    guest::FS_LIMIT,
    guest::FS_ACCESS_RIGHTS,
];
const GS: [u32; 4] = [
    guest::GS_SELECTOR,
    guest::GS_BASE,
    guest::GS_LIMIT,
    guest::GS_ACCESS_RIGHTS,
];
const SS: [u32; 4] = [
    guest::SS_SELECTOR,
    guest::SS_BASE,
    guest::SS_LIMIT,
    guest::SS_ACCESS_RIGHTS,
];
const TR: [u32; 4] = [
    guest::TR_SELECTOR,
    guest::TR_BASE,
    guest::TR_LIMIT,
    guest::TR_ACCESS_RIGHTS,
];
const LDTR: [u32; 4] = [
    guest::LDTR_SELECTOR,
    guest::LDTR_BASE,
    guest::LDTR_LIMIT,
    guest::LDTR_ACCESS_RIGHTS,
];
