// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::vm::{
        GuestContext, GuestExitInfo, VcpuArchState, VcpuMsrs, VcpuRegs, VcpuSregs, VmxExitReason,
    },
    irq::DisabledLocalIrqGuard,
    vm::GuestModeHooks,
};

use super::ioctl::{GetRegs, GetSregs, KvmRegs, KvmSregs, SetRegs, SetSregs};
use crate::{
    device::kvm::ioctl,
    dispatch_ioctl,
    kvm::{run_page::KvmRunPage, vcpu::Vcpu},
    prelude::*,
    process::{posix_thread::AsPosixThread, signal::HandlePendingSignal},
    util::ioctl::RawIoctl,
};

pub(in crate::kvm) struct VcpuArch {
    state: Mutex<VcpuState>,

    exception: SpinLock<Option<(u32, u32)>>,
}

impl VcpuArch {
    pub(in crate::kvm) fn new() -> Result<Self> {
        Ok(Self {
            state: Mutex::new(VcpuState {
                guest_context: Vcpu::new_guest_context()?,
                pending_io: None,
            }),
            exception: SpinLock::new(None),
        })
    }

    pub(in crate::kvm) fn state(&self) -> MutexGuard<'_, VcpuState> {
        self.state.lock()
    }
}

pub(in crate::kvm) struct VcpuState {
    pub guest_context: GuestContext,
    pub pending_io: Option<PendingIo>,
}

pub(in crate::kvm) struct PendingIo {
    rip: usize,
    instruction_len: u32,
}

impl VcpuState {
    fn skip_emulated_instruction(&mut self, instruction_len: u32) {
        let orig_rip = self.guest_context.regs().rip;
        let mut rip = orig_rip.wrapping_add(instruction_len as usize);
        if ((rip ^ orig_rip) >> 31) == 3 && !is_64_bit_code(self.guest_context.sregs()) {
            rip = rip as u32 as usize;
        }

        self.guest_context.regs_mut().rip = rip;
    }
}

macro_rules! write_run_fields {
    ($run_page:ident; $( [$offset:expr, $value:expr] ),+ $(,)?) => {
        $( $run_page.write_run_val($offset, &$value)?; )+
    };
}

impl Vcpu {
    pub(crate) fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetRegs => {
                let regs = KvmRegs::from(*self.arch().state().guest_context.regs());
                cmd.write(&regs)?;
                Ok(0)
            }
            cmd @ SetRegs => {
                let regs = cmd.read()?;
                *self.arch().state().guest_context.regs_mut() = regs.into();
                Ok(0)
            }
            cmd @ GetSregs => {
                let sregs = KvmSregs::from(*self.arch().state().guest_context.sregs());
                cmd.write(&sregs)?;
                Ok(0)
            }
            cmd @ SetSregs => {
                let sregs = cmd.read()?;
                *self.arch().state().guest_context.sregs_mut() = sregs.into();
                Ok(0)
            }
            _ => {
                return_errno_with_message!(Errno::ENOTTY, "unsupported vCPU ioctl")
            }
        })
    }

    fn new_guest_context() -> Result<GuestContext> {
        Ok(GuestContext::new(
            VcpuRegs::new(),
            VcpuSregs::new_real_mode(),
            VcpuMsrs::default(),
        )?)
    }

    /// Handles a VM exit and updates the guest state and run page.
    ///
    /// Returns `Ok(true)` to return to userspace, `Ok(false)` to resume the guest,
    /// or an error if the exit cannot be handled.
    pub(in crate::kvm) fn handle_exit(
        &self,
        state: &mut VcpuState,
        exit_info: GuestExitInfo,
        run_page: &KvmRunPage,
    ) -> Result<bool> {
        match VmxExitReason::try_from(exit_info.exit_reason) {
            Ok(VmxExitReason::IO_INSTRUCTION) => {
                const IO_IN: u64 = 1 << 3;
                const IO_STRING: u64 = 1 << 4;
                const IO_REP: u64 = 1 << 5;
                const PIO_DATA_OFFSET: usize = PAGE_SIZE;

                if exit_info.exit_qualification & (IO_IN | IO_STRING | IO_REP) != 0 {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "only scalar port output is supported currently"
                    );
                }

                let size = ((exit_info.exit_qualification & 7) + 1) as u8;
                run_page.write_run_bytes(
                    PIO_DATA_OFFSET,
                    &state.guest_context.regs().rax.to_le_bytes()[..usize::from(size)],
                )?;
                write_run_fields!(run_page;
                    [ioctl::KVM_RUN_IO_DIRECTION_OFFSET, ioctl::KVM_EXIT_IO_OUT],
                    [ioctl::KVM_RUN_IO_SIZE_OFFSET, size],
                    [ioctl::KVM_RUN_IO_PORT_OFFSET, (exit_info.exit_qualification >> 16) as u16],
                    [ioctl::KVM_RUN_IO_COUNT_OFFSET, 1u32],
                    [ioctl::KVM_RUN_IO_DATA_OFFSET_OFFSET, PIO_DATA_OFFSET as u64],
                    [ioctl::KVM_RUN_EXIT_REASON_OFFSET, ioctl::KVM_EXIT_IO],
                );

                // Complete the instruction when userspace re-enters `KVM_RUN`.
                state.pending_io = Some(PendingIo {
                    rip: state.guest_context.regs().rip,
                    instruction_len: exit_info.instruction_len,
                });
                Ok(true)
            }
            Ok(VmxExitReason::EPT_VIOLATION) => {
                self.vm()
                    .memory()
                    .handle_page_fault(exit_info.guest_phys_addr)?;
                Ok(false)
            }
            Ok(VmxExitReason::HLT) => {
                state.skip_emulated_instruction(exit_info.instruction_len);
                run_page.write_run_val(ioctl::KVM_RUN_EXIT_REASON_OFFSET, &ioctl::KVM_EXIT_HLT)?;
                Ok(true)
            }
            Ok(VmxExitReason::PREEMPTION_TIMER) => Ok(false),
            _ => return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported VM exit"),
        }
    }

    pub(in crate::kvm) fn finish_pending(&self, state: &mut VcpuState) {
        if let Some(pending_io) = state.pending_io.take() {
            let rip = state.guest_context.regs().rip;
            if rip == pending_io.rip {
                state.skip_emulated_instruction(pending_io.instruction_len);
            }
        }
    }
}

impl GuestModeHooks for Vcpu {
    fn pre_guest_run(&self, state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {
        // TODO: 添加时钟源后，需要在这里继续设置 `state.timer_deadline`

        state.interruption_info = if is_exception(state.idt_vectoring_info) {
            (state.idt_vectoring_info, state.idt_vectoring_error_code)
        } else if is_exception(state.pending_interruption_info.0) {
            state.pending_interruption_info
        } else if self.arch().exception.lock().is_some() {
            self.arch().exception.take()
        } else {
            (0, 0)
        };
        
        state.request_immediate_exit = false;
        state.request_interrupt_window = false;
        if is_exception(state.interruption_info.0) {
            state.pending_interruption_info = state.interruption_info;
            if is_interrupt(state.pending_interruption_info.0)
                || is_interrupt(state.idt_vectoring_info) {
                if state.is_interruptible {
                    state.request_immediate_exit = true;
                } else {
                    state.request_interrupt_window = true;
                }
            }
            return ;
        }

        if is_interrupt(state.pending_interruption_info.0)
            || is_interrupt(state.idt_vectoring_info) {
            if state.is_interruptible {
                state.interruption_info = if is_interrupt(state.pending_interruption_info.0) {
                    state.pending_interruption_info
                } else {
                    (state.idt_vectoring_info, 0)
                };
                state.pending_interruption_info = state.interruption_info;
            } else {
                state.request_interrupt_window = true;
            }
        }
    }

    fn post_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn has_kernel_event(&self) -> bool {
        current_thread!().as_posix_thread().unwrap().has_pending()
    }
}

fn is_64_bit_code(sregs: &VcpuSregs) -> bool {
    use x86_64::registers::model_specific::EferFlags;

    let long_mode_active = sregs.efer & EferFlags::LONG_MODE_ACTIVE.bits() != 0;
    let long_code_segment = sregs.cs.l & 1 != 0;
    long_mode_active && long_code_segment
}

fn is_exception(interruption_info: u32) -> bool {
    interruption_info & (1 << 31) != 0 && ((interruption_info >> 8) & 0x7) == 3
}

fn is_interrupt(interruption_info: u32) -> bool {
    interruption_info & (1 << 31) != 0 && ((interruption_info >> 8) & 0x7) == 0
}
