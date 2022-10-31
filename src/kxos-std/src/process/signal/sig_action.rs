use super::{constants::*, sig_mask::SigMask, sig_num::SigNum};
use bitflags::bitflags;
use kxos_frame::warn;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SigAction {
    Dfl, // Default action
    Ign, // Ignore this signal
    User {
        // User-given handler
        handler_addr: usize,
        flags: SigActionFlags,
        restorer_addr: usize,
        mask: SigMask,
    },
}

impl Default for SigAction {
    fn default() -> Self {
        SigAction::Dfl
    }
}

bitflags! {
    pub struct SigActionFlags: u32 {
        const SA_NOCLDSTOP  = 1;
        const SA_NOCLDWAIT  = 2;
        const SA_SIGINFO    = 4;
        const SA_ONSTACK    = 0x08000000;
        const SA_RESTART    = 0x10000000;
        const SA_NODEFER    = 0x40000000;
        const SA_RESETHAND  = 0x80000000;
        const SA_RESTORER   = 0x04000000;
    }
}

impl TryFrom<u32> for SigActionFlags {
    type Error = &'static str;

    fn try_from(bits: u32) -> Result<Self, Self::Error> {
        let flags = SigActionFlags::from_bits(bits).ok_or_else(|| "invalid sigaction flags")?;
        if flags.contains(SigActionFlags::SA_RESTART) {
            warn!("SA_RESTART is not supported");
        }
        Ok(flags)
    }
}

impl SigActionFlags {
    pub fn to_u32(&self) -> u32 {
        self.bits()
    }
}

/// The default action to signals
#[derive(Debug, Copy, Clone)]
pub enum SigDefaultAction {
    Term, // Default action is to terminate the process.
    Ign,  // Default action is to ignore the signal.
    Core, // Default action is to terminate the process and dump core (see core(5)).
    Stop, // Default action is to stop the process.
    Cont, // Default action is to continue the process if it is currently stopped.
}

impl SigDefaultAction {
    pub fn from_signum(num: SigNum) -> SigDefaultAction {
        match num {
            SIGABRT | // = SIGIOT
            SIGBUS  |
            SIGFPE  |
            SIGILL  |
            SIGQUIT |
            SIGSEGV |
            SIGSYS  | // = SIGUNUSED
            SIGTRAP |
            SIGXCPU |
            SIGXFSZ
                => SigDefaultAction::Core,
            SIGCHLD |
            SIGURG  |
            SIGWINCH
                => SigDefaultAction::Ign,
            SIGCONT
                => SigDefaultAction::Cont,
            SIGSTOP |
            SIGTSTP |
            SIGTTIN |
            SIGTTOU
                => SigDefaultAction::Stop,
            _
                => SigDefaultAction::Term,
        }
    }
}
