use ostd::arch::vm::*;

use crate::kvm::{
    ioctl::*,
    arch::ioctl::*,
    arch::mmio::*,
    arch::KvmExitReturn,
    run_page::KvmRun,
};
use ostd::mm::VmIo;
use crate::context::current_userspace;
use crate::util::ioctl::RawIoctl;

use crate::prelude::*;
use crate::dispatch_ioctl;

pub(in crate::kvm) struct VcpuArch {
    inner: SpinLock<VcpuArchInner>,

    /// MMIO instruction details
	mmio_decode: SpinLock<MmioDecode>,
}

impl VcpuArch {
    pub(in crate::kvm) fn new() -> Self {
        Self {
            inner: SpinLock::new(VcpuArchInner::new()),
            mmio_decode: SpinLock::new(MmioDecode::default()),
        }
    }

    pub(in crate::kvm) fn init(&self) {
        self.inner.lock().init();
    }

    pub(in crate::kvm) fn get_onereg(&self, reg_id: usize) -> usize {
        self.inner.lock().get_one_reg(reg_id)
    }

    pub(in crate::kvm) fn set_onereg(&self, reg_id: usize, reg_val: usize) {
        self.inner.lock().set_one_reg(reg_id, reg_val)
    }

    pub(in crate::kvm) fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetRegList => {
                let mut list = cmd.read()?;
                let n = list.n;
                list.n = self.inner.lock().num_regs();
                cmd.write(&list)?;
                if n < list.n {
                    return Err(Error::new(Errno::E2BIG));
                }
                let mut reg_list = Vec::new();
                self.inner.lock().copy_reg_vec(Some(&mut reg_list));
                reg_list.insert(0, list.n);
                current_userspace!().write_slice(raw_ioctl.arg(), &reg_list)?;
                Ok(0)
            }
            cmd @ SetMpState => {
                // TODO: impl this
                let mut _state = cmd.read()?;
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

    pub(in crate::kvm) fn run(&self, run: Arc<KvmRun>, page_tabe_root: u64) -> Result<()> {
        let at_least_once = self.inner.lock().is_ran_at_least_once();
        if !at_least_once {
            self.inner.lock().config_ran_once();
        }

        // Mark this VCPU ran at least once
	    self.inner.lock().ran_at_least_once();

        match run.read_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET)? {
            KVM_EXIT_MMIO => self.mmio_return(run.clone())?,
            KVM_EXIT_RISCV_SBI => todo!("EXIT_SBI_RETURN is not impled!"),
            KVM_EXIT_RISCV_CSR => todo!("EXIT_CSR_RETURN is not impled!"),
            _ => 0
        };

        if run.read_run_val::<u8>(KVM_RUN_IMMEDIATE_EXIT_OFFSET)? != 0 {
            return Err(Error::new(Errno::EINTR));
        }

        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_UNKNOWN)?;

        self.inner.lock().vcpu_load(page_tabe_root);

        let mut ret = KvmExitReturn::ContinueRun;
        loop {
            if ret == KvmExitReturn::ExitToUserSpace {
                break;
            }

            let guard = ostd::irq::disable_local();

            self.inner.lock().flush_interrupts();

            self.inner.lock().update_hvip();

            let trap = self.enter_exit();

            self.inner.lock().sync_interrupts();

            drop(guard);

            ret = self.exit(run.clone(), trap)?
        }

        self.inner.lock().vcpu_put();

        Ok(())
    }

    fn virtual_insn(&self, trap: &KvmCpuTrap) -> Result<KvmExitReturn> {
        let insn = trap.stval;

        let ret = match (insn & 0x007c) >> 2 {
            INSN_OPCODE_SYSTEM => handle_system_opcode_insn(insn),
            _ => todo!("virtual_insn: Other insns is not impled!"),
        };

        self.handle_insn_return(insn, ret);

        Ok(KvmExitReturn::ContinueRun)
    }

    fn handle_insn_return(&self, insn: usize, ret: KvmInsnReturn) {
        match ret {
            KvmInsnReturn::KvmInsnContinueNextSepc => {
			    self.inner.lock().guest_context.step_sepc(insn_len(insn));
            }
            _ => todo!("handle_insn_return: Other insn return is not impled!"),
        }
    }

    fn mmio_load(&self, run: Arc<KvmRun>, fault_addr: usize, htinst: usize) -> Result<KvmExitReturn> {
        self.mmio_decode.lock().reset_with_htinst(htinst);
        self.mmio_decode.lock().load_decode();

        run.write_run_val::<u8>(KVM_RUN_MMIO_IS_WRITE_OFFSET, &0)?;
        run.write_run_val::<usize>(KVM_RUN_MMIO_PHYS_ADDR_OFFSET, &fault_addr)?;
        run.write_run_val::<u32>(KVM_RUN_MMIO_LEN_OFFSET, &(self.mmio_decode.lock().len as u32))?;
        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_MMIO)?;

        Ok(KvmExitReturn::ExitToUserSpace)
    }

    fn mmio_store(&self, run: Arc<KvmRun>, fault_addr: usize, htinst: usize) -> Result<KvmExitReturn> {
        self.mmio_decode.lock().reset_with_htinst(htinst);
        let data = self.mmio_decode.lock().store_decode(&self.inner.lock().guest_context);

        match self.mmio_decode.lock().len {
            1 => run.write_run_val::<u8>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u8))?,
            2 => run.write_run_val::<u16>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u16))?,
            4 => run.write_run_val::<u32>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u32))?,
            8 => run.write_run_val::<u64>(KVM_RUN_MMIO_DATA_OFFSET, &(data as u64))?,
            _ => return Err(Error::new(Errno::EOPNOTSUPP)),
        }

        run.write_run_val::<u8>(KVM_RUN_MMIO_IS_WRITE_OFFSET, &1)?;
        run.write_run_val::<usize>(KVM_RUN_MMIO_PHYS_ADDR_OFFSET, &fault_addr)?;
        run.write_run_val::<u32>(KVM_RUN_MMIO_LEN_OFFSET, &(self.mmio_decode.lock().len as u32))?;
        run.write_run_val::<u32>(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_MMIO)?;

        Ok(KvmExitReturn::ExitToUserSpace)
    }

    fn gstage_page_fault(&self, run: Arc<KvmRun>, trap: KvmCpuTrap) -> Result<KvmExitReturn> {
        let fault_addr = (trap.htval << 2) | (trap.stval & 0x3);

        match trap.scause {
            EXC_LOAD_GUEST_PAGE_FAULT => self.mmio_load(run, fault_addr, trap.htinst),
            EXC_STORE_GUEST_PAGE_FAULT => self.mmio_store(run, fault_addr, trap.htinst),
            _ => todo!("gstage_page_fault: Other scause({:#x}) in addr({:#x}) is not impled!", trap.scause, fault_addr),
        }
    }

    fn exit(&self, run: Arc<KvmRun>, trap: KvmCpuTrap) -> Result<KvmExitReturn> {
        /* If we got host interrupt then do nothing */
	    if trap.scause & 0x1 << 63 != 0 {
            return Ok(KvmExitReturn::ContinueRun);
        }
        run.write_run_val(KVM_RUN_EXIT_REASON_OFFSET, &KVM_EXIT_UNKNOWN)?;
        
        let mut ret = Err(Error::new(Errno::EFAULT));
        match trap.scause {
            EXC_VIRTUAL_INST_FAULT => {
                if self.inner.lock().guest_context.get_hstatus() & HSTATUS_SPV != 0 {
                    ret = self.virtual_insn(&trap);
                }
            }
            EXC_INST_GUEST_PAGE_FAULT | EXC_LOAD_GUEST_PAGE_FAULT | EXC_STORE_GUEST_PAGE_FAULT => {
                if self.inner.lock().guest_context.get_hstatus() & HSTATUS_SPV != 0 {
                    ret = self.gstage_page_fault(run, trap);
                }
            }
            EXC_SUPERVISOR_SYSCALL => {
                if self.inner.lock().guest_context.get_hstatus() & HSTATUS_SPV != 0 {
                    // 后面看看怎么从ostd的error转一下
                    if let Ok(_) = self.inner.lock().sbi_call() {
                        ret = Ok(KvmExitReturn::ContinueRun)
                    }
                }
            }
            _ => todo!("VcpuArch exit: Trap({:#x}) is not impled!", trap.scause),
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
            let insn = self.mmio_decode.lock().insn;
            let len = self.mmio_decode.lock().len;
            let shift = self.mmio_decode.lock().shift;
            let index = (insn >> 7) & 0x1F;
            match len {
                1 => {
                    let data = run.read_run_val::<u8>(KVM_RUN_MMIO_DATA_OFFSET)?;
                    let wdata = ((((data as usize) << shift) as isize) >> shift) as usize;
                    self.inner.lock().guest_context.set_reg(index, wdata);
                }
                2 => {
                    let data = run.read_run_val::<u16>(KVM_RUN_MMIO_DATA_OFFSET)?;
                    let wdata = ((((data as usize) << shift) as isize) >> shift) as usize;
                    self.inner.lock().guest_context.set_reg(index, wdata);
                }
                4 => {
                    let data = run.read_run_val::<u32>(KVM_RUN_MMIO_DATA_OFFSET)?;
                    let wdata = ((((data as usize) << shift) as isize) >> shift) as usize;
                    self.inner.lock().guest_context.set_reg(index, wdata);
                }
                8 => {
                    let data = run.read_run_val::<u64>(KVM_RUN_MMIO_DATA_OFFSET)?;
                    let wdata = ((((data as usize) << shift) as isize) >> shift) as usize;
                    self.inner.lock().guest_context.set_reg(index, wdata);
                }
                _ => return Err(Error::new(Errno::EOPNOTSUPP)),
            }
        }
        let insn_len = self.mmio_decode.lock().insn_len as usize;
        self.inner.lock().guest_context.step_sepc(insn_len);
        Ok(0)
    }

    fn enter_exit(&self) -> KvmCpuTrap {
        self.inner.lock().swap_in_guest_state();

        let trap = self.inner.lock().enter_exit();

        self.inner.lock().swap_in_host_state();

        trap
    }
}
