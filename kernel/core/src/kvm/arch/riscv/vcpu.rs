// SPDX-License-Identifier: MPL-2.0

use ostd::arch::vm::{VcpuArchState, GuestExitInfo, CAUSE_IRQ_FLAG};
use super::insn::{INSN_OPCODE_MASK, INSN_OPCODE_SHIFT, KvmInsnReturn};

use crate::kvm::{
    ioctl::{KVM_RUN_EXIT_REASON_OFFSET, KVM_RUN_IMMEDIATE_EXIT_OFFSET, KVM_EXIT_UNKNOWN, KVM_EXIT_MMIO, KVM_EXIT_RISCV_SBI, 
        KVM_EXIT_RISCV_CSR, KVM_RUN_MMIO_IS_WRITE_OFFSET, INSN_OPCODE_SYSTEM, KVM_RUN_MMIO_PHYS_ADDR_OFFSET, KVM_RUN_MMIO_LEN_OFFSET,
        KVM_RUN_MMIO_DATA_OFFSET, EXC_LOAD_GUEST_PAGE_FAULT, EXC_STORE_GUEST_PAGE_FAULT, EXC_VIRTUAL_INST_FAULT, EXC_INST_GUEST_PAGE_FAULT,
        EXC_SUPERVISOR_SYSCALL},
    arch::ioctl::{GetRegList, Interrupt},
    arch::mmio::MmioDecode,
    arch::KvmExitReturn,
    run_page::KvmRun,
};
use ostd::arch::vm::{GuestContext, RiscvGuestHooks};
use ostd::vm::{GuestMode, GuestReturnReason, GuestPhysMemSpace};
use ostd::mm::VmIo;
use crate::context::current_userspace;
use crate::util::ioctl::RawIoctl;
use super::isa::KvmRiscvIsaExtId;
use super::sbi::*;

use crate::prelude::*;
use crate::dispatch_ioctl;

pub(in crate::kvm) struct VcpuArch {
    /// whether VCPU ran at least once
    pub ran_atleast_once: SpinLock<bool>,
    /// The ostd part of VcpuArch
    pub inner: SpinLock<VcpuArchState>,

    /// MMIO instruction details
	pub mmio_decode: SpinLock<MmioDecode>,

    /// ISA feature bits (similar to MISA)
    pub isa: SpinLock<u128>,

    /// SBI context
	pub sbi_context: SpinLock<VcpuSbiContext>,
}

impl VcpuArch {
    pub(in crate::kvm) fn new() -> Self {
        Self {
            ran_atleast_once: SpinLock::new(false),
            inner: SpinLock::new(VcpuArchState::default()),
            mmio_decode: SpinLock::new(MmioDecode::default()),
            isa: SpinLock::new(0),
            sbi_context: SpinLock::new(VcpuSbiContext::new()),
        }
    }

    pub(in crate::kvm) fn init(&self) -> Result<()> {
        // Setup ISA features available to VCPU
        self.kvm_riscv_vcpu_setup_isa()?;
        
        self.inner.lock().init();

        // Setup VCPU timer
        self.inner.lock().timer.init();

        // Setup SBI extensions
	    // NOTE: This must be the last thing to be initialized.
        self.sbi_context.lock().init()?;

        // Reset VCPU
        self.reset(false)?;

        Ok(())
    }

    fn reset(&self, _kvm_sbi_reset: bool) -> Result<()> {
        self.inner.lock().context_reset();
        self.inner.lock().fp_reset();
        self.inner.lock().timer.reset();
        reset_sbi(self)?;
        Ok(())
    }

    /// Mark the vCPU as having run at least once, and return whether
    /// this call was the first time it has run.
    ///
    /// Returns `true` on the first run, `false` on every subsequent run.
    pub fn mark_ran_once(&self) -> bool {
        let first_run = !*self.ran_atleast_once.lock();
        *self.ran_atleast_once.lock() = true;
        first_run
    }

    pub(in crate::kvm) fn get_onereg(&self, reg_id: usize) -> Result<usize> {
        Ok(self.get_one_reg(reg_id)?)
    }

    pub(in crate::kvm) fn set_onereg(&self, reg_id: usize, reg_val: usize) -> Result<()> {
        Ok(self.set_one_reg(reg_id, reg_val)?)
    }

    pub(super) fn kvm_riscv_vcpu_setup_isa(&self) -> Result<()> {
        for raw in 0..KvmRiscvIsaExtId::COUNT {
            let ext = KvmRiscvIsaExtId::try_from(raw)?;

            if let Ok(isa_ext) = super::isa::kvm_riscv_isa_check_host(ext) {
                if super::isa::kvm_riscv_isa_enable_allowed(isa_ext) {
                    *self.isa.lock() |= 0x1 << (isa_ext.bits().trailing_zeros() as usize);
                }
            }
        }
        Ok(())
    }

    pub(in crate::kvm) fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetRegList => {
                // First, we need to check whether the buffer size provided 
                // by the user is sufficient.
                let mut list = cmd.read()?;
                let n = list.n;
                // Get the actually required buffer size and write to user.
                list.n = self.num_regs()?;
                cmd.write(&list)?;
                // If the size provided by the user is smaller than the actual
                // required size, a response is returned informing the user 
                // that the buffer needs to be enlarged.
                if n < list.n {
                    return Err(Error::new(Errno::E2BIG));
                }
                // The size provided by the user meets the requirements
                // so we copy the regs to the user.
                let mut reg_list = Vec::new();
                self.copy_reg_vec(Some(&mut reg_list))?;
                // We need to write the entire `RegList`, 
                // so we need to prepend `n` to the `vec`.
                reg_list.insert(0, list.n);
                current_userspace!().write_slice(raw_ioctl.arg(), &reg_list)?;
                Ok(0)
            }
            cmd @ Interrupt => {
                let irq = cmd.read()?;
                self.inner.lock().set_interrupt(irq.irq);
                Ok(0)
            }
            _ => {
                let ioctl_nr = raw_ioctl.cmd() & 0xff;
                error!(
                    "kvm: unimplemented VCPU ioctl command: cmd={:#x}, nr={:#x}",
                    raw_ioctl.cmd(),
                    ioctl_nr
                );
                return_errno_with_message!(Errno::ENOTTY, "unknown VCPU ioctl command");
            }
        })
    }

    pub(in crate::kvm) fn run(&self, run: Arc<KvmRun>, guest_mem: &GuestPhysMemSpace) -> Result<()> {
        let first_run = self.mark_ran_once();
        if first_run {
            self.inner.lock().config_ran_once();
        }

        // Process the data returned after being handled by the user.
        match run.read_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET)? {
            KVM_EXIT_MMIO => self.mmio_return(run.clone())?,
            KVM_EXIT_RISCV_SBI => return_errno_with_message!(Errno::EINVAL, "EXIT_SBI_RETURN is not impled!"),
            KVM_EXIT_RISCV_CSR => return_errno_with_message!(Errno::EINVAL, "EXIT_CSR_RETURN is not impled!"),
            _ => 0
        };

        // Check whether we need to exit immediately.
        if run.read_run_val::<u8>(KVM_RUN_IMMEDIATE_EXIT_OFFSET)? != 0 {
            return Err(Error::new(Errno::EINTR));
        }

        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_UNKNOWN)?;

        self.inner.lock().vcpu_load();

        let mut ret = KvmExitReturn::ContinueRun;
        loop {
            if ret == KvmExitReturn::ExitToUserSpace {
                break;
            }

            // Bug Fix: should sync changes to self.inner
            let state = *self.inner.lock();
            let mut guest_context = GuestContext::new(state);
            let guest_mode = GuestMode::new()?;

            let reason = guest_mode.execute(&mut guest_context, guest_mem, &RiscvGuestHooks)?;

            *self.inner.lock() = guest_context.arch().clone();
            drop(guest_context);

            if let GuestReturnReason::VmExit(info) = reason {
                // Handle exit trap
                ret = self.exit(run.clone(), info)?
            }
        }

        self.inner.lock().vcpu_put();

        Ok(())
    }

    fn virtual_insn(&self, trap: &GuestExitInfo) -> Result<KvmExitReturn> {
        let insn = trap.virtual_instruction();

        let ret = match (insn & INSN_OPCODE_MASK) >> INSN_OPCODE_SHIFT {
            INSN_OPCODE_SYSTEM => super::insn::handle_system_opcode_insn(insn),
            _ => return_errno_with_message!(Errno::EINVAL, "virtual_insn: Other insns is not impled!"),
        };

        self.handle_insn_return(insn, ret)?;

        Ok(KvmExitReturn::ContinueRun)
    }

    fn handle_insn_return(&self, insn: usize, ret: KvmInsnReturn) -> Result<()> {
        match ret {
            KvmInsnReturn::ContinueNextSepc => self.inner.lock().guest_context.sepc += super::insn::insn_len(insn),
            _ => return_errno_with_message!(Errno::EINVAL, "handle_insn_return: Other insn return is not impled!"),
        }
        Ok(())
    }

    /// Emulate MMIO load instruction
    fn mmio_load(&self, run: Arc<KvmRun>, fault_addr: usize, trap_insn: usize) -> Result<KvmExitReturn> {
        self.mmio_decode.lock().reset_with_insn(trap_insn);
        self.mmio_decode.lock().load_decode();

        // Update MMIO details in kvm_run struct
        run.write_run_val::<u8>(KVM_RUN_MMIO_IS_WRITE_OFFSET, &0)?;
        run.write_run_val::<usize>(KVM_RUN_MMIO_PHYS_ADDR_OFFSET, &fault_addr)?;
        run.write_run_val::<u32>(KVM_RUN_MMIO_LEN_OFFSET, &(self.mmio_decode.lock().len as u32))?;
        
        // Exit to userspace for MMIO emulation
        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_MMIO)?;

        Ok(KvmExitReturn::ExitToUserSpace)
    }

    /// Emulate MMIO store instruction
    fn mmio_store(&self, run: Arc<KvmRun>, fault_addr: usize, trap_insn: usize) -> Result<KvmExitReturn> {
        self.mmio_decode.lock().reset_with_insn(trap_insn);
        let data = self.mmio_decode.lock().store_decode(self)?;

        // Copy data to kvm_run instance
        match self.mmio_decode.lock().len {
            1 => run.write_run_val::<u8>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u8))?,
            2 => run.write_run_val::<u16>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u16))?,
            4 => run.write_run_val::<u32>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u32))?,
            8 => run.write_run_val::<u64>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u64))?,
            _ => return Err(Error::new(Errno::EOPNOTSUPP)),
        }

        // Update MMIO details in kvm_run struct
        run.write_run_val::<u8>(KVM_RUN_MMIO_IS_WRITE_OFFSET, &1)?;
        run.write_run_val::<usize>(KVM_RUN_MMIO_PHYS_ADDR_OFFSET, &fault_addr)?;
        run.write_run_val::<u32>(KVM_RUN_MMIO_LEN_OFFSET, &(self.mmio_decode.lock().len as u32))?;
        
        // Exit to userspace for MMIO emulation
        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_MMIO)?;

        Ok(KvmExitReturn::ExitToUserSpace)
    }

    fn gstage_page_fault(&self, run: Arc<KvmRun>, trap: GuestExitInfo) -> Result<KvmExitReturn> {
        let fault_addr = trap.fault_addr();

        match trap.trap_type() {
            EXC_LOAD_GUEST_PAGE_FAULT => self.mmio_load(run, fault_addr, trap.trap_instruction()),
            EXC_STORE_GUEST_PAGE_FAULT => self.mmio_store(run, fault_addr, trap.trap_instruction()),
            _ => return_errno_with_message!(Errno::EINVAL, "gstage_page_fault: Other scause is not impled!"),
        }
    }

    fn exit(&self, run: Arc<KvmRun>, trap: GuestExitInfo) -> Result<KvmExitReturn> {
        // If we got host interrupt then do nothing
	    if trap.trap_type() & CAUSE_IRQ_FLAG != 0 {
            return Ok(KvmExitReturn::ContinueRun);
        }
        run.write_run_val(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_UNKNOWN)?;
        
        let mut ret = Err(Error::new(Errno::EFAULT));
        match trap.trap_type() {
            EXC_VIRTUAL_INST_FAULT => {
                if self.inner.lock().trap_is_from_guest() {
                    ret = self.virtual_insn(&trap);
                }
            }
            EXC_INST_GUEST_PAGE_FAULT | EXC_LOAD_GUEST_PAGE_FAULT | EXC_STORE_GUEST_PAGE_FAULT => {
                if self.inner.lock().trap_is_from_guest() {
                    ret = self.gstage_page_fault(run, trap);
                }
            }
            EXC_SUPERVISOR_SYSCALL => {
                if self.inner.lock().trap_is_from_guest() {
                    ret = Ok(super::sbi::sbi_ecall(&self)?.into());
                }
            }
            _ => return_errno_with_message!(Errno::EINVAL, "VcpuArch exit: Trap is not impled!"),
        }
        ret
    }

    /// Process MMIO value returned from user-space
    fn mmio_return(&self, run: Arc<KvmRun>) -> Result<usize> {
        if self.mmio_decode.lock().return_handled == 1 {
            return Ok(0);
        }
        self.mmio_decode.lock().return_handled = 1;

        if run.read_run_val::<u8>(KVM_RUN_MMIO_IS_WRITE_OFFSET)? != 1 {
            self.mmio_decode.lock().handle_return(run, self)?;
        }
        
        let insn_len = self.mmio_decode.lock().insn_len as usize;
        self.inner.lock().guest_context.sepc += insn_len;
        Ok(0)
    }
}
