// SPDX-License-Identifier: MPL-2.0

//! Common x86 register transactions after backend-specific allocation.

use x86::msr::{rdmsr, wrmsr};

use super::{amd, intel};
use crate::pmu::{EventConfig, PmuCaps, Result};

const EVTSEL_ENABLE: u64 = 1 << 22;
const INTEL_FIXED_CTRL_STRIDE: u32 = 4;
const INTEL_FIXED_CTRL_MASK: u64 = 0xf;
const MSR_IA32_PERF_GLOBAL_OVF_CTRL: u32 = 0x390;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    IntelFixed,
    IntelGeneral,
    AmdCore,
}

#[derive(Debug)]
pub(crate) enum Caps {
    Intel(intel::Caps),
    Amd(amd::Caps),
}

impl Caps {
    pub(crate) fn summary(&self) -> PmuCaps {
        match self {
            Self::Intel(caps) => caps.summary(),
            Self::Amd(caps) => caps.summary(),
        }
    }
}

pub(crate) fn configure(
    config: EventConfig,
    caps: &Caps,
    used: &[Option<usize>],
) -> Result<(Token, u64)> {
    let mut token = match caps {
        Caps::Intel(caps) => intel::allocate(config, caps, used)?,
        Caps::Amd(caps) => amd::allocate(config, caps, used)?,
    };
    if let Some(msr) = token.global_msr {
        // SAFETY: Allocation selected the global control MSR for this CPU. Only
        // this token's gate is cleared, and access is serialized by the caller.
        unsafe {
            let control = rdmsr(msr);
            token.saved_gate = control & token.global_bit;
            wrmsr(msr, control & !token.global_bit);
        }
    }
    token.program_control(token.config_bits);
    let initial_raw = token.read();
    Ok((token, initial_raw))
}

/// An owned x86-64 hardware counter register allocation.
#[derive(Debug)]
pub(crate) struct Token {
    pub(super) kind: Kind,
    pub(super) index: u32,
    pub(super) width_bits: u8,
    pub(super) control_msr: u32,
    pub(super) counter_msr: u32,
    pub(super) global_msr: Option<u32>,
    pub(super) global_bit: u64,
    pub(super) saved_gate: u64,
    pub(super) config_bits: u64,
    pub(super) overflow_msr: Option<u32>,
}

impl Token {
    pub(crate) fn index(&self) -> usize {
        self.index as usize
            + if self.kind == Kind::IntelFixed {
                256
            } else {
                0
            }
    }

    pub(crate) fn width_bits(&self) -> u8 {
        self.width_bits
    }

    pub(crate) fn read(&self) -> u64 {
        // SAFETY: The token identifies a counter enumerated on the owning CPU,
        // where the caller serializes access with local IRQs disabled.
        unsafe { rdmsr(self.counter_msr) }
    }

    pub(crate) fn start(&self) -> u64 {
        let initial_raw = self.read();
        // SAFETY: The token owns the selected counter. Only its local enable bit
        // and its global gate are set on the owning CPU.
        unsafe {
            if self.kind != Kind::IntelFixed {
                wrmsr(self.control_msr, self.config_bits | EVTSEL_ENABLE);
            }
            if let Some(global_msr) = self.global_msr {
                update_msr(global_msr, 0, self.global_bit);
            }
        }
        initial_raw
    }

    pub(crate) fn stop(&self) -> u64 {
        self.stop_gates();
        self.read()
    }

    fn stop_gates(&self) {
        // SAFETY: Both writes disable only the gate of the counter owned by
        // this token and run on the owning CPU.
        unsafe {
            if self.kind != Kind::IntelFixed {
                wrmsr(self.control_msr, self.config_bits);
            }
            if let Some(global_msr) = self.global_msr {
                update_msr(global_msr, self.global_bit, 0);
            }
        }
    }

    pub(crate) fn release(&self) {
        self.stop_gates();
        self.program_control(0);
        if let Some(global_msr) = self.global_msr {
            // SAFETY: Clearing the event control leaves the counter disabled.
            // Restore only this token's saved global bit on the owning CPU.
            unsafe { update_msr(global_msr, self.global_bit, self.saved_gate) };
        }
    }

    fn program_control(&self, bits: u64) {
        // SAFETY: The token holds the control registers enumerated for this CPU.
        // The callers pass either its validated event control bits or zero, and
        // overflow clearing targets only this counter's status bit.
        unsafe {
            if self.kind == Kind::IntelFixed {
                let shift = self.index * INTEL_FIXED_CTRL_STRIDE;
                update_msr(self.control_msr, INTEL_FIXED_CTRL_MASK << shift, bits);
                wrmsr(MSR_IA32_PERF_GLOBAL_OVF_CTRL, self.global_bit);
            } else {
                wrmsr(self.control_msr, bits);
                if let Some(msr) = self.overflow_msr {
                    wrmsr(msr, self.global_bit);
                }
            }
        }
    }
}

/// # Safety
///
/// `msr` must be a readable and writable PMU control register on the current CPU.
/// The masks must change only bits owned by the caller, who must serialize access.
unsafe fn update_msr(msr: u32, clear_mask: u64, set_mask: u64) {
    // SAFETY: The caller guarantees register access and valid masks.
    unsafe { wrmsr(msr, (rdmsr(msr) & !clear_mask) | set_mask) };
}
