// SPDX-License-Identifier: MPL-2.0

//! VMCS execution controls and host/guest state synchronization.

use ::x86::{
    msr, segmentation,
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
    vmx::{VmxGuard, context_switch, vmcs::CurrentVmcs},
    x86,
};
use crate::{
    arch::trap::{gdt, idt},
    irq::DisabledLocalIrqGuard,
    prelude::*,
};

pub(super) const REQUIRED_PINBASED_CONTROLS: u32 = PinbasedControls::EXTERNAL_INTERRUPT_EXITING
    .bits()
    | PinbasedControls::NMI_EXITING.bits()
    | PinbasedControls::VMX_PREEMPTION_TIMER.bits();
pub(super) const REQUIRED_PRIMARY_CONTROLS: u32 = PrimaryControls::HLT_EXITING.bits()
    | PrimaryControls::UNCOND_IO_EXITING.bits()
    | PrimaryControls::SECONDARY_CONTROLS.bits();
pub(super) const REQUIRED_SECONDARY_CONTROLS: u32 =
    SecondaryControls::ENABLE_EPT.bits() | SecondaryControls::UNRESTRICTED_GUEST.bits();
pub(super) const REQUIRED_EXIT_CONTROLS: u32 = ExitControls::HOST_ADDRESS_SPACE_SIZE.bits()
    | ExitControls::SAVE_IA32_PAT.bits()
    | ExitControls::LOAD_IA32_PAT.bits()
    | ExitControls::SAVE_IA32_EFER.bits()
    | ExitControls::LOAD_IA32_EFER.bits();
pub(super) const REQUIRED_ENTRY_CONTROLS: u32 =
    EntryControls::LOAD_IA32_PAT.bits() | EntryControls::LOAD_IA32_EFER.bits();

macro_rules! write_vmcs_fields {
    ($vmcs:ident; $( [$field:expr, $value:expr] ),+ $(,)?) => {{
        $( $vmcs.write($field, $value as usize)?; )+
        Result::<()>::Ok(())
    }};
}

macro_rules! read_vmcs_fields {
    ($vmcs:ident; $( [$field:expr, $dest:expr] ),+ $(,)?) => {
        $( $dest = $vmcs.read($field)? as _; )+
    };
}

impl CurrentVmcs<'_> {
    /// Initializes the fixed execution controls.
    pub(super) fn setup_fixed_controls(&self, _vmx_guard: &VmxGuard) -> Result<()> {
        // SAFETY: `_vmx_guard` guarantees VMX support on every CPU.
        let has_true_controls = unsafe { msr::rdmsr(msr::IA32_VMX_BASIC) } & (1 << 55) != 0;

        let set_control_fn = |field, capability_msr, required: u32| {
            // SAFETY: Call sites use supported VMX control MSRs, selecting true
            // controls only when `has_true_controls` is set.
            let capability = unsafe { msr::rdmsr(capability_msr) };
            let value = (required | capability as u32) & (capability >> 32) as u32;

            // SAFETY: Call sites use the controls validated when acquiring `_vmx_guard`.
            unsafe { self.write(field, value as usize) }
        };

        set_control_fn(
            control::PINBASED_EXEC_CONTROLS,
            if has_true_controls {
                msr::IA32_VMX_TRUE_PINBASED_CTLS
            } else {
                msr::IA32_VMX_PINBASED_CTLS
            },
            REQUIRED_PINBASED_CONTROLS,
        )?;
        set_control_fn(
            control::PRIMARY_PROCBASED_EXEC_CONTROLS,
            if has_true_controls {
                msr::IA32_VMX_TRUE_PROCBASED_CTLS
            } else {
                msr::IA32_VMX_PROCBASED_CTLS
            },
            REQUIRED_PRIMARY_CONTROLS,
        )?;
        set_control_fn(
            control::SECONDARY_PROCBASED_EXEC_CONTROLS,
            msr::IA32_VMX_PROCBASED_CTLS2,
            REQUIRED_SECONDARY_CONTROLS,
        )?;
        set_control_fn(
            control::VMEXIT_CONTROLS,
            if has_true_controls {
                msr::IA32_VMX_TRUE_EXIT_CTLS
            } else {
                msr::IA32_VMX_EXIT_CTLS
            },
            REQUIRED_EXIT_CONTROLS,
        )?;
        set_control_fn(
            control::VMENTRY_CONTROLS,
            if has_true_controls {
                msr::IA32_VMX_TRUE_ENTRY_CTLS
            } else {
                msr::IA32_VMX_ENTRY_CTLS
            },
            REQUIRED_ENTRY_CONTROLS,
        )?;

        // SAFETY: Exceptions remain in the guest and MSR lists are disabled.
        unsafe {
            write_vmcs_fields!(self;
                [control::EXCEPTION_BITMAP, 0],
                [control::PAGE_FAULT_ERR_CODE_MASK, 0],
                [control::PAGE_FAULT_ERR_CODE_MATCH, 0],
                [control::CR3_TARGET_COUNT, 0],

                [control::VMEXIT_MSR_STORE_COUNT, 0],
                [control::VMEXIT_MSR_LOAD_COUNT, 0],
                [control::VMENTRY_MSR_LOAD_COUNT, 0],
            )
        }
    }

    /// Synchronizes the guest execution mode and EPT pointer.
    ///
    /// # Safety
    ///
    /// `eptp` must identify a valid EPT that isolates guest memory from the host.
    /// Page-table and backing frames must remain alive while the guest can access them.
    pub(super) unsafe fn sync_controls(&self, eptp: u64, guest_efer: u64) -> Result<()> {
        let mut entry = self.read(control::VMENTRY_CONTROLS)?
            & !(EntryControls::IA32E_MODE_GUEST.bits() as usize);
        if guest_efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0 {
            entry |= EntryControls::IA32E_MODE_GUEST.bits() as usize;
        }

        // SAFETY: `VmxGuard` validates both guest modes, and the caller guarantees
        // EPT isolation and frame lifetimes.
        unsafe {
            write_vmcs_fields!(self;
                [control::VMENTRY_CONTROLS, entry],
                [control::EPTP_FULL, eptp],
            )
        }
    }

    /// Initializes host state that is stable on this CPU.
    pub(super) fn setup_fixed_host(&self) -> Result<()> {
        // SAFETY: These fields restore the kernel's code segment, CPU-local base,
        // MSRs and descriptor tables, and return to the VM-exit handler.
        unsafe {
            write_vmcs_fields!(self;
                [host::CS_SELECTOR, gdt::KERNEL_CS.0],
                [host::GS_BASE, msr::rdmsr(msr::IA32_GS_BASE)],

                [host::IA32_PAT_FULL, msr::rdmsr(msr::IA32_PAT)],
                [host::IA32_EFER_FULL, msr::rdmsr(msr::IA32_EFER)],
                [host::IA32_SYSENTER_CS, 0],
                [host::IA32_SYSENTER_ESP, 0],
                [host::IA32_SYSENTER_EIP, 0],

                [host::TR_SELECTOR, gdt::TSS_SELECTOR.0],
                [host::TR_BASE, gdt::tss_base(self.irq_guard())],
                [host::GDTR_BASE, gdt::gdt_base(self.irq_guard())],
                [host::IDTR_BASE, idt::idt_base()],

                [host::RIP, context_switch::vm_exit_handler_virtaddr()],
            )
        }
    }

    /// Refreshes host state for a VM exit on the current task.
    pub(super) fn sync_host(&self) -> Result<()> {
        // SAFETY: These fields restore the host's live control registers, selectors and FS base.
        unsafe {
            write_vmcs_fields!(self;
                [host::CR0, Cr0::read_raw()],
                [host::CR3, x86::read_cr3_raw()],
                [host::CR4, Cr4::read_raw()],

                [host::ES_SELECTOR, segmentation::es().bits()],
                [host::SS_SELECTOR, segmentation::ss().bits()],
                [host::DS_SELECTOR, segmentation::ds().bits()],
                [host::FS_SELECTOR, segmentation::fs().bits()],
                [host::GS_SELECTOR, segmentation::gs().bits()],
                [host::FS_BASE, msr::rdmsr(msr::IA32_FS_BASE)],
            )
        }
    }

    /// Loads guest architectural state into the VMCS.
    pub(super) fn load_guest_context(
        &self,
        arch: &VcpuArchState,
        vmx_guard: &VmxGuard,
    ) -> Result<()> {
        let sregs = &arch.sregs;

        // SAFETY: These writes do not affect host state.
        unsafe {
            self.write_segment(&sregs.cs, CS)?;
            self.write_segment(&sregs.ds, DS)?;
            self.write_segment(&sregs.es, ES)?;
            self.write_segment(&sregs.fs, FS)?;
            self.write_segment(&sregs.gs, GS)?;
            self.write_segment(&sregs.ss, SS)?;
            self.write_segment(&sregs.tr, TR)?;
            self.write_segment(&sregs.ldt, LDTR)?;

            write_vmcs_fields!(self;
                [guest::CR0, fix_guest_cr0(sregs.cr0, vmx_guard, self.irq_guard())],
                [guest::CR4, fix_guest_cr4(sregs.cr4, vmx_guard, self.irq_guard())],
                [control::CR0_GUEST_HOST_MASK, usize::MAX],
                [control::CR4_GUEST_HOST_MASK, usize::MAX],
                [control::CR0_READ_SHADOW, sregs.cr0],
                [control::CR4_READ_SHADOW, sregs.cr4],
                [guest::CR3, sregs.cr3],

                [guest::GDTR_BASE, sregs.gdt.base],
                [guest::GDTR_LIMIT, sregs.gdt.limit],
                [guest::IDTR_BASE, sregs.idt.base],
                [guest::IDTR_LIMIT, sregs.idt.limit],

                [guest::RIP, arch.regs.rip],
                [guest::RSP, arch.regs.rsp],
                [guest::RFLAGS, arch.regs.rflags | 2],

                [guest::IA32_EFER_FULL, sregs.efer],
                [guest::IA32_PAT_FULL, arch.msrs.pat],
                [guest::IA32_SYSENTER_CS, arch.msrs.sysenter_cs],
                [guest::IA32_SYSENTER_ESP, arch.msrs.sysenter_esp],
                [guest::IA32_SYSENTER_EIP, arch.msrs.sysenter_eip],

                // HLT exits are handled in software, so every explicit entry is active.
                [guest::ACTIVITY_STATE, 0],
                [guest::PENDING_DBG_EXCEPTIONS, 0],
                [guest::LINK_PTR_FULL, usize::MAX],
            )
        }
    }

    /// Saves VMCS guest state into the architectural context.
    pub(super) fn save_guest_context(&self, arch: &mut VcpuArchState) -> Result<()> {
        let mut sregs = arch.sregs;
        let mut regs = arch.regs;

        sregs.cs = self.read_segment(CS)?;
        sregs.ds = self.read_segment(DS)?;
        sregs.es = self.read_segment(ES)?;
        sregs.fs = self.read_segment(FS)?;
        sregs.gs = self.read_segment(GS)?;
        sregs.ss = self.read_segment(SS)?;
        sregs.tr = self.read_segment(TR)?;
        sregs.ldt = self.read_segment(LDTR)?;

        read_vmcs_fields!(self;
            [guest::GDTR_BASE, sregs.gdt.base],
            [guest::GDTR_LIMIT, sregs.gdt.limit],
            [guest::IDTR_BASE, sregs.idt.base],
            [guest::IDTR_LIMIT, sregs.idt.limit],
            [guest::CR3, sregs.cr3],

            [guest::RIP, regs.rip],
            [guest::RSP, regs.rsp],
            [guest::RFLAGS, regs.rflags],
        );

        arch.sregs = sregs;
        arch.regs = regs;
        Ok(())
    }

    /// Writes a guest segment's VMCS fields.
    fn write_segment(
        &self,
        segment: &VcpuSegment,
        [selector, base, limit, rights]: [u32; 4],
    ) -> Result<()> {
        // SAFETY: Call sites supply guest segment fields, which do not
        // affect the host state restored on VM exit.
        unsafe {
            write_vmcs_fields!(self;
                [selector, segment.selector],
                [base, segment.base],
                [limit, segment.limit],
                [rights, segment.vmcs_access_rights()],
            )
        }
    }

    /// Reads a guest segment's VMCS fields.
    fn read_segment(&self, [selector, base, limit, rights]: [u32; 4]) -> Result<VcpuSegment> {
        let mut segment = VcpuSegment::default();
        let access: usize;

        read_vmcs_fields!(self;
            [rights, access],
            [base, segment.base],
            [limit, segment.limit],
            [selector, segment.selector],
        );

        segment.set_vmcs_access_rights(access);
        Ok(segment)
    }
}

impl VcpuSegment {
    fn vmcs_access_rights(&self) -> usize {
        let mut access = usize::from(self.type_ & 0xf);
        access |= usize::from(self.s & 1) << 4;
        access |= usize::from(self.dpl & 3) << 5;
        access |= usize::from(self.present & 1) << 7;
        access |= usize::from(self.avl & 1) << 12;
        access |= usize::from(self.l & 1) << 13;
        access |= usize::from(self.db & 1) << 14;
        access |= usize::from(self.g & 1) << 15;
        access |= usize::from(self.unusable & 1) << 16;
        access
    }

    fn set_vmcs_access_rights(&mut self, access: usize) {
        self.type_ = (access & 0xf) as u8;
        self.present = ((access >> 7) & 1) as u8;
        self.dpl = ((access >> 5) & 3) as u8;
        self.db = ((access >> 14) & 1) as u8;
        self.s = ((access >> 4) & 1) as u8;
        self.l = ((access >> 13) & 1) as u8;
        self.g = ((access >> 15) & 1) as u8;
        self.avl = ((access >> 12) & 1) as u8;
        self.unusable = ((access >> 16) & 1) as u8;
    }
}

fn fix_guest_cr0(cr0: u64, _vmx_guard: &VmxGuard, _irq_guard: &DisabledLocalIrqGuard) -> u64 {
    // SAFETY: `_vmx_guard` guarantees VMX support on every CPU.
    let (fixed0, fixed1) = unsafe {
        (
            msr::rdmsr(msr::IA32_VMX_CR0_FIXED0),
            msr::rdmsr(msr::IA32_VMX_CR0_FIXED1),
        )
    };
    // Unrestricted guests need not have PE or PG set. The read shadow retains
    // the architectural value, including bits forced by VMX requirements.
    let fixed0 = fixed0 & !(Cr0Flags::PROTECTED_MODE_ENABLE | Cr0Flags::PAGING).bits();
    (cr0 | fixed0) & fixed1
}

fn fix_guest_cr4(cr4: u64, _vmx_guard: &VmxGuard, _irq_guard: &DisabledLocalIrqGuard) -> u64 {
    // SAFETY: `_vmx_guard` guarantees VMX support on every CPU.
    let (fixed0, fixed1) = unsafe {
        (
            msr::rdmsr(msr::IA32_VMX_CR4_FIXED0),
            msr::rdmsr(msr::IA32_VMX_CR4_FIXED1),
        )
    };
    (cr4 | fixed0) & fixed1
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
