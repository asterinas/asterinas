// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::cpu::context::{CpuException, PageFaultErrorCode, UserContext},
    user::UserContextApi,
};

use crate::{
    process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal},
    thread::exception::ToFaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_rdi(sig_num.as_u8() as usize);
        self.set_rsi(siginfo_addr);
        self.set_rdx(ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use crate::process::signal::constants::*;

        let rip = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            CpuException::DivisionError => (SIGFPE, FPE_INTDIV, Some(rip)),
            CpuException::Debug => {
                // TODO: Derive the code from the debug status.
                (SIGTRAP, TRAP_TRACE, Some(rip))
            }
            CpuException::BreakPoint => {
                // Linux uses `SI_KERNEL` without an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/traps.c#L998>
                (SIGTRAP, SI_KERNEL, None)
            }
            CpuException::Overflow => {
                // Linux uses `SI_KERNEL` without an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/traps.c#L387>
                (SIGSEGV, SI_KERNEL, None)
            }
            CpuException::InvalidOpcode => (SIGILL, ILL_ILLOPN, Some(rip)),
            CpuException::StackSegmentFault(..) => {
                // Linux uses `SI_KERNEL` without an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/traps.c#L519-L520>
                (SIGBUS, SI_KERNEL, None)
            }
            CpuException::GeneralProtectionFault(..) => {
                // Linux uses `SI_KERNEL` without an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/traps.c#L904-L911>
                (SIGSEGV, SI_KERNEL, None)
            }
            CpuException::PageFault(raw_page_fault_info) => {
                // FIXME: The code should depend on whether the faulting address is covered by a
                // mapping, not just on the `PRESENT` bit.
                let code = if raw_page_fault_info
                    .error_code
                    .contains(PageFaultErrorCode::PRESENT)
                {
                    SEGV_ACCERR
                } else {
                    SEGV_MAPERR
                };
                let addr = Some(raw_page_fault_info.addr as u64);
                (SIGSEGV, code, addr)
            }
            CpuException::X87FloatingPointException | CpuException::SIMDFloatingPointException => {
                // TODO: Derive the code from the floating-point status.
                (SIGFPE, FPE_FLTDIV, Some(rip))
            }
            CpuException::AlignmentCheck => {
                // Linux does not provide an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/traps.c#L538-L539>
                (SIGBUS, BUS_ADRALN, None)
            }

            _ => return None,
        };

        Some(FaultSignal::new(num, code, addr))
    }
}
