/// implements GuestMode
///
mod gpm_space;
mod host_context;
mod interrupt;
mod timer;

use host_context::*;
use x86::msr::*;

pub use self::{
    gpm_space::GuestPhysMemSpace, interrupt::GuestInterruptPort, timer::GuestTimerPort,
};
use crate::{
    Error,
    arch::vm::{
        context::{
            GuestContext, VcpuControlRegister, VcpuControlRegisters, VcpuDtable, VcpuRunState,
            VcpuSegment,
        },
        exit::GuestExitInfo,
        vmx::{
            Msr, VmcsControl32, VmcsControl64, VmcsControlNW, VmcsGuest16, VmcsGuest32,
            VmcsGuest64, VmcsGuestNW, VmcsReadOnly32, exit_info,
        },
        x86::write_cr2_raw,
    },
    mm::Gpaddr,
    prelude::*,
    sync::{Mutex, SpinLock},
};

pub struct GuestMode<'a> {
    context: &'a Mutex<GuestContext>,
    interrupt_port: &'a SpinLock<dyn GuestInterruptPort>,
    timer_port: &'a SpinLock<dyn GuestTimerPort>,
}

/// Translates a guest virtual address to a guest physical address.
pub fn translate_gva_to_gpa(
    context: &GuestContext,
    guest_mem: &GuestPhysMemSpace,
    gva: usize,
) -> Result<Gpaddr> {
    const PTE_PRESENT: u64 = 1 << 0;
    const PTE_HUGE: u64 = 1 << 7;
    const PTE_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;
    const PAGE_2M_MASK: Gpaddr = (1 << 21) - 1;
    const PAGE_1G_MASK: Gpaddr = (1 << 30) - 1;
    const PTE_SIZE: Gpaddr = core::mem::size_of::<u64>();

    let cr0 = context.arch().cr0();
    let cr3 = context.arch().cr3();
    if (cr0 & (1 << 31)) == 0 {
        return Ok(gva);
    }

    let read_guest_pte = |gpa: Gpaddr| -> Result<u64> {
        let mut reader = guest_mem.reader(gpa, PTE_SIZE)?;
        reader.read_val::<u64>()
    };
    let pte_addr = |entry: u64| -> Gpaddr { (entry & PTE_ADDR_MASK) as Gpaddr };

    let cr3 = (cr3 as Gpaddr) & !0xfff;
    let pml4e_gpa = cr3 + (((gva >> 39) & 0x1ff) * PTE_SIZE);
    let pml4e = read_guest_pte(pml4e_gpa)?;
    if (pml4e & PTE_PRESENT) == 0 {
        return Err(Error::PageFault);
    }

    let pdpte = read_guest_pte(pte_addr(pml4e) + (((gva >> 30) & 0x1ff) * PTE_SIZE))?;
    if (pdpte & PTE_PRESENT) == 0 {
        return Err(Error::PageFault);
    }
    if (pdpte & PTE_HUGE) != 0 {
        return Ok(pte_addr(pdpte) | (gva & PAGE_1G_MASK));
    }

    let pde = read_guest_pte(pte_addr(pdpte) + (((gva >> 21) & 0x1ff) * PTE_SIZE))?;
    if (pde & PTE_PRESENT) == 0 {
        return Err(Error::PageFault);
    }
    if (pde & PTE_HUGE) != 0 {
        return Ok(pte_addr(pde) | (gva & PAGE_2M_MASK));
    }

    let pte = read_guest_pte(pte_addr(pde) + (((gva >> 12) & 0x1ff) * PTE_SIZE))?;
    if (pte & PTE_PRESENT) == 0 {
        return Err(Error::PageFault);
    }

    Ok(pte_addr(pte) | (gva & 0xfff))
}

impl<'a> GuestMode<'a> {
    pub fn new(
        context: &'a Mutex<GuestContext>,
        interrupt_port: &'a SpinLock<dyn GuestInterruptPort>,
        timer_port: &'a SpinLock<dyn GuestTimerPort>,
    ) -> Self {
        GuestMode {
            context,
            interrupt_port,
            timer_port,
        }
    }

    pub fn execute(&mut self, eptp: u64) -> Result<GuestExitInfo> {
        if self.context.lock().run_state() == VcpuRunState::WaitForSipi {
            return self.wait_for_sipi(self.context.lock().arch().rip() as _);
        }

        // VMCS state is per-pCPU while loaded. Keep this run on one pCPU, then
        // clear the VMCS before returning so the next RSH_RUN may migrate safely.
        let _preempt_guard = crate::task::disable_preempt();
        let _run_guard = self.enter_run(eptp)?;

        loop {
            let irq_guard = crate::irq::disable_local();

            let host_context = self.prepare_vmentry()?;
            let run_result = self.vmlaunch_or_vmresume();
            self.complete_vmexit(host_context, run_result)?;

            use crate::arch::vm::exit::vmexit_handler;
            let exit_info = exit_info().map_err(Error::from)?;
            let exit_info = vmexit_handler(self.context, &exit_info)?;
            drop(irq_guard);

            // Deliver handling of vmexit to kernel client or userspace.
            if let Some(exit_info) = exit_info {
                return Ok(exit_info);
            }
        }
    }

    fn enter_run(&self, eptp: u64) -> Result<GuestRunGuard<'a>> {
        self.init_vmcs(eptp)?;

        // Set running state in guest_context.
        let mut context = self.context.lock();
        if context.run_state() == VcpuRunState::Runnable {
            context.set_running();
        } else {
            error!("unexpected run state.");
        }
        Ok(GuestRunGuard {
            guest_context: self.context,
        })
    }

    fn init_vmcs(&self, eptp: u64) -> Result<()> {
        self.context.lock().vmcs.load()?;
        if self.context.lock().vmcs.initialized() {
            return Ok(());
        }

        debug!("rustshyper: initializing vcpu vmcs");
        let mut context = self.context.lock();
        let vmcs_guest_state = context.vmcs_guest_state();
        context.vmcs.init(vmcs_guest_state, eptp as u64)?;

        Ok(())
    }

    fn prepare_vmentry(&self) -> Result<HostContext> {
        self.prepare_preemption_timer()?;
        self.prepare_interrupt()?;
        let host_context = HostContext::save();
        if let Err(err) = self.load_guest_context() {
            host_context.load();
            return Err(err);
        }
        Ok(host_context)
    }

    fn vmlaunch_or_vmresume(&self) -> Result<()> {
        let launched: u64 = if self.context.lock().vmcs.launched() {
            1
        } else {
            0
        };

        use crate::arch::vm::vmx::vcpu_run;
        let mut context = self.context.lock();
        let ret = vcpu_run(context.arch_mut().regs_mut_ptr(), launched);
        if ret != 0 {
            log_vcpu_run_failure(launched);
            return Err(Error::InvalidArgs);
        }

        context.vmcs.set_launched(true);
        Ok(())
    }

    fn complete_vmexit(&self, host_context: HostContext, run_result: Result<()>) -> Result<()> {
        let save_guest_context_result = self.save_guest_context();
        host_context.load();

        run_result?;
        save_guest_context_result?;

        // self.context.

        Ok(())
    }

    fn prepare_interrupt(&self) -> Result<Option<u8>> {
        VmcsControl32::VMENTRY_INTERRUPTION_INFO_FIELD
            .write(0)
            .map_err(Error::from)?;

        let pending_vector = self.interrupt_port.lock().check_pending_interrupt();

        let Some(vector) = pending_vector else {
            return Ok(None);
        };

        // why?
        if vector < 32 {
            return Ok(None);
        }

        if self.context.lock().after_hlt {
            clear_block_by_sti()?;
            VmcsGuest32::ACTIVITY_STATE.write(0).map_err(Error::from)?;
            self.context.lock().after_hlt = false;
        }

        use crate::arch::vm::interrupt::*;
        let intr_info = u32::from(vector) | INTR_INFO_VALID_MASK | INTR_TYPE_EXT_INTR;
        let injectable = vmx_interrupt_injectable()?;

        if !injectable {
            enable_interrupt_window_exiting()?;
            return Ok(None);
        }
        disable_interrupt_window_exiting()?;

        // inject interrupt through VMCS
        VmcsControl32::VMENTRY_INTERRUPTION_INFO_FIELD
            .write(intr_info)
            .map_err(Error::from)?;
        self.interrupt_port.lock().accept_interrupt(vector);
        return Ok(Some(vector));
    }

    fn prepare_preemption_timer(&self) -> Result<()> {
        let context = self.context.lock();
        let vcpu_id = context.cpu_config.vcpu_id;
        let guest_tsc = context.guest_tsc();
        let msr_deadline = context
            .tsc_deadline()
            .filter(|deadline| *deadline > guest_tsc);
        let timer_deadline = self.timer_port.lock().check_deadline(guest_tsc);
        let msr_gap = msr_deadline.map(|deadline| deadline.saturating_sub(guest_tsc).max(1));
        let timer_gap = timer_deadline.map(|deadline| deadline.saturating_sub(guest_tsc).max(1));
        let gap = min_gap(msr_gap, timer_gap).unwrap_or(500_000);
        let timer_value = vmx_preemption_timer_ticks(gap);
        VmcsGuest32::VMX_PREEMPTION_TIMER_VALUE.write(timer_value)?;
        VmcsControl64::TSC_OFFSET.write(context.tsc_offset as u64)?;
        Ok(())
    }

    fn load_guest_context(&self) -> Result<()> {
        let mut context = self.context.lock();
        let cr2 = context.arch().cr2();
        write_cr2_raw(cr2);
        self.load_guest_run_msrs(&context);
        context.arch_mut().load_fpu();

        VmcsGuestNW::RIP
            .write(context.arch().rip() as usize)
            .map_err(Error::from)?;
        VmcsGuestNW::RSP
            .write(context.arch().gpr(7) as usize)
            .map_err(Error::from)?;
        // TODO: why | 0x2 ?
        VmcsGuestNW::RFLAGS
            .write((context.arch().rflags() | 0x2) as usize)
            .map_err(Error::from)?;

        write_control_registers_to_vmcs(context.arch().control_regs())?;

        use x86::{msr::*, vmx::vmcs::control::EntryControls};
        use x86_64::registers::model_specific::EferFlags;
        let guest_efer = context.arch().msr(IA32_EFER);
        VmcsGuest64::IA32_EFER
            .write(guest_efer)
            .map_err(Error::from)?;
        let mut entry = VmcsControl32::VMENTRY_CONTROLS
            .read()
            .map_err(Error::from)?;
        if guest_efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0 {
            entry |= EntryControls::IA32E_MODE_GUEST.bits();
        } else {
            entry &= !EntryControls::IA32E_MODE_GUEST.bits();
        }
        VmcsControl32::VMENTRY_CONTROLS
            .write(entry)
            .map_err(Error::from)?;

        let guest_cr3 = context.arch().cr3();
        VmcsGuestNW::CR3
            .write(guest_cr3 as usize)
            .map_err(Error::from)?;

        VmcsGuest64::IA32_PAT
            .write(context.arch().msr(IA32_PAT))
            .map_err(Error::from)?;
        VmcsGuestNW::FS_BASE
            .write(context.arch().msr(IA32_FS_BASE) as usize)
            .map_err(Error::from)?;
        VmcsGuestNW::GS_BASE
            .write(context.arch().msr(IA32_GS_BASE) as usize)
            .map_err(Error::from)?;
        VmcsGuest32::IA32_SYSENTER_CS
            .write(context.arch().msr(IA32_SYSENTER_CS) as u32)
            .map_err(Error::from)?;
        VmcsGuestNW::IA32_SYSENTER_ESP
            .write(context.arch().msr(IA32_SYSENTER_ESP) as usize)
            .map_err(Error::from)?;
        VmcsGuestNW::IA32_SYSENTER_EIP
            .write(context.arch().msr(IA32_SYSENTER_EIP) as usize)
            .map_err(Error::from)?;

        Ok(())
    }

    fn save_guest_context(&self) -> Result<()> {
        self.context.lock().arch_mut().save_fpu();
        self.save_guest_run_msrs()?;
        use x86_64::registers::control::Cr2;
        self.context.lock().arch_mut().set_cr2(Cr2::read_raw());

        let mut context = self.context.lock();
        context
            .arch_mut()
            .set_rip(VmcsGuestNW::RIP.read().map_err(Error::from)? as u64);
        context
            .arch_mut()
            .set_gpr(7, 8, VmcsGuestNW::RSP.read().map_err(Error::from)? as u64);
        context
            .arch_mut()
            .set_rflags(VmcsGuestNW::RFLAGS.read().map_err(Error::from)? as u64);

        let guest_cr3 = VmcsGuestNW::CR3.read().map_err(Error::from)?;
        context.arch_mut().set_cr3(guest_cr3 as u64);

        context
            .arch_mut()
            .set_control_regs_from_vmcs(read_control_registers_from_vmcs()?);

        let guest_efer = VmcsGuest64::IA32_EFER.read().map_err(Error::from)?;
        context.arch_mut().set_msr(IA32_EFER, guest_efer);

        context.arch_mut().set_gdt(read_dtable_from_vmcs(
            VmcsGuestNW::GDTR_BASE,
            VmcsGuest32::GDTR_LIMIT,
        )?);
        context.arch_mut().set_idt(read_dtable_from_vmcs(
            VmcsGuestNW::IDTR_BASE,
            VmcsGuest32::IDTR_LIMIT,
        )?);

        context.arch_mut().set_cs(read_segment_from_vmcs(
            VmcsGuest16::CS_SELECTOR,
            VmcsGuestNW::CS_BASE,
            VmcsGuest32::CS_LIMIT,
            VmcsGuest32::CS_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_ds(read_segment_from_vmcs(
            VmcsGuest16::DS_SELECTOR,
            VmcsGuestNW::DS_BASE,
            VmcsGuest32::DS_LIMIT,
            VmcsGuest32::DS_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_es(read_segment_from_vmcs(
            VmcsGuest16::ES_SELECTOR,
            VmcsGuestNW::ES_BASE,
            VmcsGuest32::ES_LIMIT,
            VmcsGuest32::ES_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_fs(read_segment_from_vmcs(
            VmcsGuest16::FS_SELECTOR,
            VmcsGuestNW::FS_BASE,
            VmcsGuest32::FS_LIMIT,
            VmcsGuest32::FS_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_gs(read_segment_from_vmcs(
            VmcsGuest16::GS_SELECTOR,
            VmcsGuestNW::GS_BASE,
            VmcsGuest32::GS_LIMIT,
            VmcsGuest32::GS_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_ss(read_segment_from_vmcs(
            VmcsGuest16::SS_SELECTOR,
            VmcsGuestNW::SS_BASE,
            VmcsGuest32::SS_LIMIT,
            VmcsGuest32::SS_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_tr(read_segment_from_vmcs(
            VmcsGuest16::TR_SELECTOR,
            VmcsGuestNW::TR_BASE,
            VmcsGuest32::TR_LIMIT,
            VmcsGuest32::TR_ACCESS_RIGHTS,
        )?);
        context.arch_mut().set_ldt(read_segment_from_vmcs(
            VmcsGuest16::LDTR_SELECTOR,
            VmcsGuestNW::LDTR_BASE,
            VmcsGuest32::LDTR_LIMIT,
            VmcsGuest32::LDTR_ACCESS_RIGHTS,
        )?);

        Ok(())
    }

    // TODO: understand this two functions.
    fn load_guest_run_msrs(&self, context: &GuestContext) {
        Msr::IA32_STAR.write(context.arch().msr(IA32_STAR));
        Msr::IA32_LSTAR.write(context.arch().msr(IA32_LSTAR));
        Msr::IA32_CSTAR.write(context.arch().msr(IA32_CSTAR));
        Msr::IA32_FMASK.write(context.arch().msr(IA32_FMASK));
        Msr::IA32_KERNEL_GSBASE.write(context.arch().msr(IA32_KERNEL_GSBASE));
    }

    fn save_guest_run_msrs(&self) -> Result<()> {
        let star = Msr::IA32_STAR.read();
        let lstar = Msr::IA32_LSTAR.read();
        let cstar = Msr::IA32_CSTAR.read();
        let syscall_mask = Msr::IA32_FMASK.read();
        let kernel_gs_base = Msr::IA32_KERNEL_GSBASE.read();
        let fs_base = VmcsGuestNW::FS_BASE.read().map_err(Error::from)? as u64;
        let gs_base = VmcsGuestNW::GS_BASE.read().map_err(Error::from)? as u64;

        let mut context = self.context.lock();
        context.arch_mut().set_msr(IA32_STAR, star);
        context.arch_mut().set_msr(IA32_LSTAR, lstar);
        context.arch_mut().set_msr(IA32_CSTAR, cstar);
        context.arch_mut().set_msr(IA32_FMASK, syscall_mask);
        context
            .arch_mut()
            .set_msr(IA32_KERNEL_GSBASE, kernel_gs_base);
        context.arch_mut().set_msr(IA32_FS_BASE, fs_base);
        context.arch_mut().set_msr(IA32_GS_BASE, gs_base);
        Ok(())
    }

    fn wait_for_sipi(&self, rip: Gpaddr) -> Result<GuestExitInfo> {
        use crate::arch::vm::vmx::VmxExitReason;
        Ok(GuestExitInfo {
            exit_reason: VmxExitReason::HLT as _,
            instruction_len: 0,
            exit_qualification: 0,
            guest_phys_addr: 0,
            guest_rip: rip,
        })
    }
}

struct GuestRunGuard<'a> {
    guest_context: &'a Mutex<GuestContext>,
}

impl Drop for GuestRunGuard<'_> {
    fn drop(&mut self) {
        if let Err(err) = self.guest_context.lock().vmcs.quit() {
            error!("errno: {:?}", err);
            error!("unexpect condition: failed to quit vmcs")
        }
        self.guest_context.lock().quit_running();
    }
}

fn vmx_preemption_timer_ticks(tsc_cycles: u64) -> u32 {
    let rate = (Msr::IA32_VMX_MISC.read() & 0x1f) as u32;
    let rounding = (1_u64 << rate).saturating_sub(1);
    (tsc_cycles.saturating_add(rounding) >> rate) as u32
}

fn min_gap(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(gap), None) | (None, Some(gap)) => Some(gap),
        (None, None) => None,
    }
}

fn read_dtable_from_vmcs(base_field: VmcsGuestNW, limit_field: VmcsGuest32) -> Result<VcpuDtable> {
    Ok(VcpuDtable {
        base: base_field.read().map_err(Error::from)? as u64,
        limit: limit_field.read().map_err(Error::from)? as u16,
        padding: [0; 3],
    })
}

fn write_control_registers_to_vmcs(control_regs: VcpuControlRegisters) -> Result<()> {
    write_control_register_to_vmcs(
        control_regs.cr0(),
        VmcsGuestNW::CR0,
        VmcsControlNW::CR0_GUEST_HOST_MASK,
        VmcsControlNW::CR0_READ_SHADOW,
    )?;
    write_control_register_to_vmcs(
        control_regs.cr4(),
        VmcsGuestNW::CR4,
        VmcsControlNW::CR4_GUEST_HOST_MASK,
        VmcsControlNW::CR4_READ_SHADOW,
    )
}

fn write_control_register_to_vmcs(
    reg: VcpuControlRegister,
    real_field: VmcsGuestNW,
    mask_field: VmcsControlNW,
    shadow_field: VmcsControlNW,
) -> Result<()> {
    real_field.write(reg.real() as usize).map_err(Error::from)?;
    mask_field
        .write(reg.host_mask() as usize)
        .map_err(Error::from)?;
    shadow_field
        .write(reg.read_shadow() as usize)
        .map_err(Error::from)
}

fn read_control_registers_from_vmcs() -> Result<VcpuControlRegisters> {
    let cr0 = read_control_register_state_from_vmcs(
        VmcsGuestNW::CR0,
        VmcsControlNW::CR0_GUEST_HOST_MASK,
        VmcsControlNW::CR0_READ_SHADOW,
    )?;
    let cr4 = read_control_register_state_from_vmcs(
        VmcsGuestNW::CR4,
        VmcsControlNW::CR4_GUEST_HOST_MASK,
        VmcsControlNW::CR4_READ_SHADOW,
    )?;
    Ok(VcpuControlRegisters::from_vmcs(cr0, cr4))
}

fn read_control_register_state_from_vmcs(
    value_field: VmcsGuestNW,
    mask_field: VmcsControlNW,
    shadow_field: VmcsControlNW,
) -> Result<VcpuControlRegister> {
    let real = value_field.read().map_err(Error::from)? as u64;
    let mask = mask_field.read().map_err(Error::from)? as u64;
    let shadow = shadow_field.read().map_err(Error::from)? as u64;
    Ok(VcpuControlRegister::from_vmcs(mask, shadow, real))
}

fn read_segment_from_vmcs(
    selector_field: VmcsGuest16,
    base_field: VmcsGuestNW,
    limit_field: VmcsGuest32,
    rights_field: VmcsGuest32,
) -> Result<VcpuSegment> {
    let rights = rights_field.read().map_err(Error::from)?;
    Ok(VcpuSegment {
        base: base_field.read().map_err(Error::from)? as u64,
        limit: limit_field.read().map_err(Error::from)?,
        selector: selector_field.read().map_err(Error::from)?,
        type_: (rights & 0x0f) as u8,
        s: ((rights >> 4) & 0x1) as u8,
        dpl: ((rights >> 5) & 0x3) as u8,
        present: ((rights >> 7) & 0x1) as u8,
        avl: ((rights >> 12) & 0x1) as u8,
        l: ((rights >> 13) & 0x1) as u8,
        db: ((rights >> 14) & 0x1) as u8,
        g: ((rights >> 15) & 0x1) as u8,
        unusable: ((rights >> 16) & 0x1) as u8,
        padding: 0,
    })
}

fn log_vcpu_run_failure(launched: u64) {
    error!(
        "rustshyper: vcpu_run failed, launched={} vm_instruction_error={:?} \
             guest_rip={:?} guest_rsp={:?} guest_rflags={:?} guest_cr0={:?} \
             guest_cr3={:?} guest_cr4={:?} guest_efer={:?} pin_ctls={:?} \
             primary_ctls={:?} secondary_ctls={:?} exit_ctls={:?} entry_ctls={:?} \
             eptp={:?}",
        launched,
        VmcsReadOnly32::VM_INSTRUCTION_ERROR.read().ok(),
        VmcsGuestNW::RIP.read().ok(),
        VmcsGuestNW::RSP.read().ok(),
        VmcsGuestNW::RFLAGS.read().ok(),
        VmcsGuestNW::CR0.read().ok(),
        VmcsGuestNW::CR3.read().ok(),
        VmcsGuestNW::CR4.read().ok(),
        VmcsGuest64::IA32_EFER.read().ok(),
        VmcsControl32::PINBASED_EXEC_CONTROLS.read().ok(),
        VmcsControl32::PRIMARY_PROCBASED_EXEC_CONTROLS.read().ok(),
        VmcsControl32::SECONDARY_PROCBASED_EXEC_CONTROLS.read().ok(),
        VmcsControl32::VMEXIT_CONTROLS.read().ok(),
        VmcsControl32::VMENTRY_CONTROLS.read().ok(),
        VmcsControl64::EPTP.read().ok(),
    );
}
