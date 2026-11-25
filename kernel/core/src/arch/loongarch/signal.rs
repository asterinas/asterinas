// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::cpu::context::{CpuException, UserContext},
    user::UserContextApi,
};

use crate::{
    process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal},
    thread::exception::ToFaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_a0(sig_num.as_u8() as usize);
        self.set_a1(siginfo_addr);
        self.set_a2(ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use CpuException::*;

        use crate::process::signal::constants::*;

        let era = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            LoadPageFault(addr) | StorePageFault(addr) | FetchPageFault(addr) => {
                // FIXME: The code should be `SEGV_ACCERR` for faults within an existing mapping.
                (SIGSEGV, SEGV_MAPERR, Some(*addr as u64))
            }
            PageModifyFault(addr)
            | PageNonReadableFault(addr)
            | PageNonExecutableFault(addr)
            | PagePrivilegeIllegal(addr) => (SIGSEGV, SEGV_ACCERR, Some(*addr as u64)),
            FetchInstructionAddressError | MemoryAccessAddressError => {
                // TODO: Report `si_addr`.
                (SIGBUS, BUS_ADRERR, None)
            }
            AddressNotAligned => {
                // TODO: Report `si_addr`.
                (SIGBUS, BUS_ADRALN, None)
            }
            BoundsCheckFault => {
                // TODO: Report `si_addr`, `si_lower`, and `si_upper`.
                (SIGSEGV, SEGV_BNDERR, None)
            }
            Breakpoint => {
                // TODO: Decode the faulting `break` instruction and choose the signal and code.
                (SIGTRAP, TRAP_BRKPT, Some(era))
            }
            InstructionNotExist | InstructionPrivilegeIllegal => {
                // Linux uses `SI_KERNEL` without an address.
                //
                // Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/loongarch/kernel/traps.c#L877>
                (SIGILL, SI_KERNEL, None)
            }
            FloatingPointUnavailable => {
                // TODO: Support FPU in LoongArch64.
                (SIGFPE, FPE_FLTINV, Some(era))
            }

            Syscall => return None,
        };

        Some(FaultSignal::new(num, code, addr))
    }
}
