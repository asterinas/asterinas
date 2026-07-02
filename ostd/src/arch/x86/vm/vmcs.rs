// SPDX-License-Identifier: MPL-2.0

use x86::{
    dtables::{self, DescriptorTablePointer},
    segmentation, task,
    vmx::vmcs::control::{
        EntryControls, ExitControls, PinbasedControls, PrimaryControls, SecondaryControls,
    },
};
use x86_64::registers::{
    control::{Cr0, Cr3, Cr4},
    model_specific::EferFlags,
};

use super::{
    control_regs::VcpuControlRegisters,
    types::{VcpuMsrs, VcpuRegs, VcpuSegment, VcpuSregs},
    vmx::*,
    x86::get_tr_base,
};
use crate::{
    mm::{Frame, FrameAllocOptions, PAGE_SIZE, VmIo},
    prelude::*,
};

pub(crate) struct Vmcs {
    /// VMCS memory region.
    vmcs_region: Frame<()>,
    /// IO bitmap A for trapping lower port range accesses.
    io_bitmap_a: Frame<()>,
    /// IO bitmap B for trapping upper port range accesses.
    io_bitmap_b: Frame<()>,
    /// MSR bitmap for trapping RDMSR/WRMSR accesses.
    msr_bitmap: Frame<()>,
    /// True for setup works before the first launch is done.
    state: VmcsState,
}

struct VmcsState {
    initialized: bool,
    loaded: bool,
    launched: bool,
}

pub(crate) struct VmcsGuestState {
    pub(crate) regs: VcpuRegs,
    pub(crate) sregs: VcpuSregs,
    pub(crate) control_regs: VcpuControlRegisters,
    pub(crate) msrs: VcpuMsrs,
}

impl VmcsGuestState {
    // fn regs(&self) -> VcpuRegs {
    //     self.regs
    // }

    // fn sregs(&self) -> VcpuSregs {
    //     self.sregs
    // }

    // fn msrs(&self) -> VcpuMsrs {
    //     self.msrs
    // }
}

impl Vmcs {
    pub fn new() -> Result<Self> {
        // Allocate VMCS
        let vmcs_region = alloc_vmcs()?;

        let io_bitmap_a = FrameAllocOptions::new().alloc_frame()?;
        let io_bitmap_b = FrameAllocOptions::new().alloc_frame()?;
        let msr_bitmap = FrameAllocOptions::new().alloc_frame()?;
        let all_ones = [0xff_u8; PAGE_SIZE];
        io_bitmap_a.write_bytes(0, &all_ones)?;
        io_bitmap_b.write_bytes(0, &all_ones)?;
        msr_bitmap.write_bytes(0, &all_ones)?;

        Ok(Self {
            vmcs_region,
            io_bitmap_a,
            io_bitmap_b,
            msr_bitmap,
            state: VmcsState {
                initialized: false,
                loaded: false,
                launched: false,
            },
        })
    }

    fn vmcs_phys(&self) -> Paddr {
        self.vmcs_region.paddr()
    }

    pub fn load(&mut self) -> Result<()> {
        vmptrld(self.vmcs_phys() as _)?;
        self.state.loaded = true;
        Ok(())
    }

    pub fn initialized(&self) -> bool {
        self.state.initialized
    }

    pub fn launched(&self) -> bool {
        self.state.launched
    }

    pub fn set_launched(&mut self, value: bool) {
        self.state.launched = value
    }

    pub fn quit(&mut self) -> Result<()> {
        vmclear(self.vmcs_phys() as u64)?;
        self.state.loaded = false;
        self.state.launched = false;
        Ok(())
    }

    pub fn init(&mut self, vmcs_guest_state: VmcsGuestState, eptp: u64) -> Result<()> {
        if !self.state.loaded {
            vmclear(self.vmcs_phys() as u64)?;
            vmptrld(self.vmcs_phys() as u64)?;
            self.state.loaded = true;
        }
        self.setup_vmcs(vmcs_guest_state, eptp)?;
        self.state.initialized = true;
        self.state.launched = false;
        Ok(())
    }

    /// Setup VMCS with initial guest state
    fn setup_vmcs(&self, vmcs_guest_state: VmcsGuestState, eptp: u64) -> Result<()> {
        self.setup_vmcs_host()?;
        self.setup_vmcs_guest(&vmcs_guest_state)?;
        self.setup_vmcs_controls(&vmcs_guest_state, eptp)?;
        Ok(())
    }

    fn setup_vmcs_host(&self) -> Result<()> {
        VmcsHost64::IA32_PAT.write(Msr::IA32_PAT.read())?;
        VmcsHost64::IA32_EFER.write(Msr::IA32_EFER.read())?;

        VmcsHostNW::CR0.write(Cr0::read_raw() as _)?;
        VmcsHostNW::CR3.write(Cr3::read_raw().0.start_address().as_u64() as _)?; // TODO: check difference with JiaYuekai
        VmcsHostNW::CR4.write(Cr4::read_raw() as _)?;

        VmcsHost16::ES_SELECTOR.write(segmentation::es().bits())?;
        VmcsHost16::CS_SELECTOR.write(segmentation::cs().bits())?;
        VmcsHost16::SS_SELECTOR.write(segmentation::ss().bits())?;
        VmcsHost16::DS_SELECTOR.write(segmentation::ds().bits())?;
        VmcsHost16::FS_SELECTOR.write(segmentation::fs().bits())?;
        VmcsHost16::GS_SELECTOR.write(segmentation::gs().bits())?;
        VmcsHostNW::FS_BASE.write(Msr::IA32_FS_BASE.read() as _)?;
        VmcsHostNW::GS_BASE.write(Msr::IA32_GS_BASE.read() as _)?;

        // SAFETY: STR only reads the current task-register selector.
        let tr = unsafe { task::tr() };
        let mut gdtp = DescriptorTablePointer::<u64>::default();
        let mut idtp = DescriptorTablePointer::<u64>::default();
        // SAFETY: SGDT/SIDT only read descriptor-table registers into local memory.
        unsafe {
            dtables::sgdt(&mut gdtp);
            dtables::sidt(&mut idtp);
        }

        VmcsHost16::TR_SELECTOR.write(tr.bits())?;
        VmcsHostNW::TR_BASE.write(get_tr_base(tr, &gdtp) as _)?;
        VmcsHostNW::GDTR_BASE.write(gdtp.base as usize)?;
        VmcsHostNW::IDTR_BASE.write(idtp.base as usize)?;
        VmcsHostNW::RIP.write(vm_exit_handler_virtaddr() as _)?;

        VmcsHostNW::IA32_SYSENTER_ESP.write(0)?;
        VmcsHostNW::IA32_SYSENTER_EIP.write(0)?;
        VmcsHost32::IA32_SYSENTER_CS.write(0)?;
        Ok(())
    }

    fn setup_vmcs_guest(&self, vmcs_guest_state: &VmcsGuestState) -> Result<()> {
        let regs = vmcs_guest_state.regs;
        let sregs = vmcs_guest_state.sregs;
        let control_regs = vmcs_guest_state.control_regs;
        let msrs = vmcs_guest_state.msrs;

        let cr0 = control_regs.cr0();
        VmcsGuestNW::CR0.write(cr0.real() as _)?;
        VmcsControlNW::CR0_GUEST_HOST_MASK.write(cr0.host_mask() as _)?;
        VmcsControlNW::CR0_READ_SHADOW.write(cr0.read_shadow() as _)?;

        let cr4 = control_regs.cr4();
        VmcsGuestNW::CR4.write(cr4.real() as _)?;
        VmcsControlNW::CR4_GUEST_HOST_MASK.write(cr4.host_mask() as _)?;
        VmcsControlNW::CR4_READ_SHADOW.write(cr4.read_shadow() as _)?;

        {
            use VmcsGuest16::*;
            use VmcsGuest32::*;
            use VmcsGuestNW::*;
            ES_SELECTOR.write(sregs.es.selector)?;
            ES_BASE.write(sregs.es.base as usize)?;
            ES_LIMIT.write(sregs.es.limit)?;
            ES_ACCESS_RIGHTS.write(segment_access_rights(&sregs.es))?;

            CS_SELECTOR.write(sregs.cs.selector)?;
            CS_BASE.write(sregs.cs.base as usize)?;
            CS_LIMIT.write(sregs.cs.limit)?;
            CS_ACCESS_RIGHTS.write(segment_access_rights(&sregs.cs))?;

            SS_SELECTOR.write(sregs.ss.selector)?;
            SS_BASE.write(sregs.ss.base as usize)?;
            SS_LIMIT.write(sregs.ss.limit)?;
            SS_ACCESS_RIGHTS.write(segment_access_rights(&sregs.ss))?;

            DS_SELECTOR.write(sregs.ds.selector)?;
            DS_BASE.write(sregs.ds.base as usize)?;
            DS_LIMIT.write(sregs.ds.limit)?;
            DS_ACCESS_RIGHTS.write(segment_access_rights(&sregs.ds))?;

            FS_SELECTOR.write(sregs.fs.selector)?;
            FS_BASE.write(sregs.fs.base as usize)?;
            FS_LIMIT.write(sregs.fs.limit)?;
            FS_ACCESS_RIGHTS.write(segment_access_rights(&sregs.fs))?;

            GS_SELECTOR.write(sregs.gs.selector)?;
            GS_BASE.write(sregs.gs.base as usize)?;
            GS_LIMIT.write(sregs.gs.limit)?;
            GS_ACCESS_RIGHTS.write(segment_access_rights(&sregs.gs))?;

            TR_SELECTOR.write(sregs.tr.selector)?;
            TR_BASE.write(sregs.tr.base as usize)?;
            TR_LIMIT.write(sregs.tr.limit)?;
            TR_ACCESS_RIGHTS.write(segment_access_rights(&sregs.tr))?;

            LDTR_SELECTOR.write(sregs.ldt.selector)?;
            LDTR_BASE.write(sregs.ldt.base as usize)?;
            LDTR_LIMIT.write(sregs.ldt.limit)?;
            LDTR_ACCESS_RIGHTS.write(segment_access_rights(&sregs.ldt))?;
        }

        VmcsGuestNW::GDTR_BASE.write(sregs.gdt.base as usize)?;
        VmcsGuest32::GDTR_LIMIT.write(sregs.gdt.limit as u32)?;
        VmcsGuestNW::IDTR_BASE.write(sregs.idt.base as usize)?;
        VmcsGuest32::IDTR_LIMIT.write(sregs.idt.limit as u32)?;

        VmcsGuestNW::CR3.write(sregs.cr3 as usize)?;
        VmcsGuestNW::DR7.write(0x400)?;
        VmcsGuestNW::RSP.write(regs.rsp as usize)?;
        VmcsGuestNW::RIP.write(regs.rip as usize)?;
        VmcsGuestNW::RFLAGS.write((regs.rflags | 0x2) as usize)?;
        VmcsGuestNW::PENDING_DBG_EXCEPTIONS.write(0)?;
        VmcsGuestNW::IA32_SYSENTER_ESP.write(msrs.sysenter_esp as usize)?;
        VmcsGuestNW::IA32_SYSENTER_EIP.write(msrs.sysenter_eip as usize)?;
        VmcsGuest32::IA32_SYSENTER_CS.write(msrs.sysenter_cs as u32)?;

        VmcsGuest32::INTERRUPTIBILITY_STATE.write(0)?;
        VmcsGuest32::ACTIVITY_STATE.write(0)?;
        VmcsGuest32::VMX_PREEMPTION_TIMER_VALUE.write(0)?;

        VmcsGuest64::LINK_PTR.write(u64::MAX)?; // SDM Vol. 3C, Section 24.4.2
        VmcsGuest64::IA32_DEBUGCTL.write(0)?;
        VmcsGuest64::IA32_PAT.write(msrs.pat)?;
        VmcsGuest64::IA32_EFER.write(msrs.efer)?;
        VmcsControl64::TSC_OFFSET.write(0)?;
        Ok(())
    }

    fn setup_vmcs_controls(&self, vmcs_guest_state: &VmcsGuestState, eptp: u64) -> Result<()> {
        set_control(
            VmcsControl32::PINBASED_EXEC_CONTROLS,
            Msr::IA32_VMX_TRUE_PINBASED_CTLS,
            Msr::IA32_VMX_PINBASED_CTLS.read() as u32,
            (PinbasedControls::EXTERNAL_INTERRUPT_EXITING
                | PinbasedControls::NMI_EXITING
                | PinbasedControls::VMX_PREEMPTION_TIMER)
                .bits(),
            0,
        )?;

        let secondary_cap = Msr::IA32_VMX_PROCBASED_CTLS2.read();
        let secondary_allowed1 = (secondary_cap >> 32) as u32;
        let supports_pause_loop_exiting =
            (secondary_allowed1 & SecondaryControls::PAUSE_LOOP_EXITING.bits()) != 0;
        let pause_exiting_fallback = if supports_pause_loop_exiting {
            0
        } else {
            PrimaryControls::PAUSE_EXITING.bits()
        };

        set_control(
            VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS,
            Msr::IA32_VMX_TRUE_PROCBASED_CTLS,
            Msr::IA32_VMX_PROCBASED_CTLS.read() as u32,
            (PrimaryControls::USE_TSC_OFFSETTING
                | PrimaryControls::HLT_EXITING
                // | PrimaryControls::RDTSC_EXITING
                | PrimaryControls::USE_IO_BITMAPS
                | PrimaryControls::USE_MSR_BITMAPS
                | PrimaryControls::SECONDARY_CONTROLS)
                .bits()
                | pause_exiting_fallback,
            (PrimaryControls::CR3_LOAD_EXITING | PrimaryControls::CR3_STORE_EXITING).bits(),
        )?;

        let pause_loop_exiting = if supports_pause_loop_exiting {
            SecondaryControls::PAUSE_LOOP_EXITING
        } else {
            SecondaryControls::empty()
        };
        set_control(
            VmcsControl32::SECONDARY_PROCBASED_EXEC_CONTROLS,
            Msr::IA32_VMX_PROCBASED_CTLS2,
            0,
            (SecondaryControls::ENABLE_EPT
                | SecondaryControls::ENABLE_RDTSCP
                | SecondaryControls::UNRESTRICTED_GUEST
                | pause_loop_exiting)
                .bits(),
            0,
        )?;
        if supports_pause_loop_exiting {
            const VMX_PAUSE_LOOP_EXIT_GAP: u32 = 1_000_000;
            const VMX_PAUSE_LOOP_EXIT_WINDOW: u32 = 4096;
            VmcsControl32::PLE_GAP.write(VMX_PAUSE_LOOP_EXIT_GAP)?;
            VmcsControl32::PLE_WINDOW.write(VMX_PAUSE_LOOP_EXIT_WINDOW)?;
        }

        set_control(
            VmcsControl32::VMEXIT_CONTROLS,
            Msr::IA32_VMX_TRUE_EXIT_CTLS,
            Msr::IA32_VMX_EXIT_CTLS.read() as u32,
            (ExitControls::HOST_ADDRESS_SPACE_SIZE
                | ExitControls::SAVE_IA32_PAT
                | ExitControls::LOAD_IA32_PAT
                | ExitControls::SAVE_IA32_EFER
                | ExitControls::LOAD_IA32_EFER)
                .bits(),
            0,
        )?;

        let mut entry_controls =
            (EntryControls::LOAD_IA32_PAT | EntryControls::LOAD_IA32_EFER).bits();
        let msrs = vmcs_guest_state.msrs;
        if msrs.efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0 {
            entry_controls |= EntryControls::IA32E_MODE_GUEST.bits();
        }

        set_control(
            VmcsControl32::VMENTRY_CONTROLS,
            Msr::IA32_VMX_TRUE_ENTRY_CTLS,
            Msr::IA32_VMX_ENTRY_CTLS.read() as u32,
            entry_controls,
            0,
        )?;

        // No MSR switches if hypervisor doesn't use and there is only one vCPU.
        VmcsControl32::VMEXIT_MSR_STORE_COUNT.write(0)?;
        VmcsControl32::VMEXIT_MSR_LOAD_COUNT.write(0)?;
        VmcsControl32::VMENTRY_MSR_LOAD_COUNT.write(0)?;

        // Pass-through exceptions. Intercept I/O and MSR accesses via bitmaps.
        VmcsControl32::EXCEPTION_BITMAP.write(0)?;
        VmcsControl64::IO_BITMAP_A_ADDR.write(self.io_bitmap_a.paddr() as u64)?;
        VmcsControl64::IO_BITMAP_B_ADDR.write(self.io_bitmap_b.paddr() as u64)?;
        VmcsControl64::MSR_BITMAPS_ADDR.write(self.msr_bitmap.paddr() as u64)?;

        // setup EPT
        VmcsControl64::EPTP.write(eptp)?;
        Ok(())
    }
}

impl Drop for Vmcs {
    fn drop(&mut self) {
        if !self.state.loaded {
            return;
        }

        if let Err(err) = vmclear(self.vmcs_phys() as u64) {
            warn!("rustshyper: failed to clear VMCS during drop: {:?}", err);
        }
    }
}

// TODO: clear up the following code.
pub(super) fn segment_access_rights(segment: &VcpuSegment) -> u32 {
    let mut rights = u32::from(segment.type_ & 0x0f);
    rights |= u32::from(segment.s & 0x1) << 4;
    rights |= u32::from(segment.dpl & 0x3) << 5;
    rights |= u32::from(segment.present & 0x1) << 7;
    rights |= u32::from(segment.avl & 0x1) << 12;
    rights |= u32::from(segment.l & 0x1) << 13;
    rights |= u32::from(segment.db & 0x1) << 14;
    rights |= u32::from(segment.g & 0x1) << 15;
    rights |= u32::from(segment.unusable & 0x1) << 16;
    rights
}
