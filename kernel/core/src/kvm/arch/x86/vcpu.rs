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
    device::hypervisor::ioctl,
    dispatch_ioctl,
    kvm::{
        run_page::KvmRunPage,
        vcpu::{Vcpu, VcpuState},
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, signal::HandlePendingSignal},
    util::ioctl::RawIoctl,
};

pub(in crate::kvm) struct PendingIo {
    rip: usize,
    instruction_len: u32,
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
                let regs = KvmRegs::from(*self.state().guest_context.regs());
                cmd.write(&regs)?;
                Ok(0)
            }
            cmd @ SetRegs => {
                let regs = cmd.read()?;
                *self.state().guest_context.regs_mut() = regs.into();
                Ok(0)
            }
            cmd @ GetSregs => {
                let sregs = KvmSregs::from(*self.state().guest_context.sregs());
                cmd.write(&sregs)?;
                Ok(0)
            }
            cmd @ SetSregs => {
                let sregs = cmd.read()?;
                *self.state().guest_context.sregs_mut() = sregs.into();
                Ok(0)
            }
            _ => {
                return_errno_with_message!(Errno::ENOTTY, "unsupported vCPU ioctl")
            }
        })
    }

    pub(in crate::kvm) fn new_guest_context() -> Result<GuestContext> {
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
                let rip = state
                    .guest_context
                    .regs()
                    .rip
                    .checked_add(exit_info.instruction_len as usize)
                    .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
                run_page.write_run_val(ioctl::KVM_RUN_EXIT_REASON_OFFSET, &ioctl::KVM_EXIT_HLT)?;
                state.guest_context.regs_mut().rip = rip;
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
                state.guest_context.regs_mut().rip += pending_io.instruction_len as usize;
            }
        }
    }
}

impl GuestModeHooks for Vcpu {
    fn pre_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn post_guest_run(&self, _state: &mut VcpuArchState, _guard: &DisabledLocalIrqGuard) {}

    fn has_kernel_event(&self) -> bool {
        current_thread!().as_posix_thread().unwrap().has_pending()
    }
}
