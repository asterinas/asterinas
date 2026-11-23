// SPDX-License-Identifier: MPL-2.0

//! AMD core PMU discovery and event matching.
//! Resource enumeration follows AMD APM volume 2, section 13.2.
//! Event encodings require an audited family/model register reference.

use x86::msr::rdmsr;

use super::counter::{Kind, Token};
use crate::pmu::{BackendKind, Event, EventConfig, PmuCaps, PmuError, Result};

const EXTENDED_CORE: u32 = 1 << 23;
const GLOBAL_CONTROL: u32 = 0xc000_0301;
const OVERFLOW_CONTROL: u32 = 0xc000_0302;
const COUNTER_WIDTH: u8 = 48;

#[derive(Debug)]
pub(crate) struct Caps {
    family: u32,
    model: u32,
    count: u8,
    has_extended_core: bool,
    has_global_control: bool,
}

pub(super) fn probe() -> Result<Caps> {
    let signature = super::cpuid(1, 0).ok_or(PmuError::Unsupported)?.eax;
    let base_family = (signature >> 8) & 0xf;
    let family = base_family
        + if base_family == 0xf {
            (signature >> 20) & 0xff
        } else {
            0
        };
    let model = ((signature >> 4) & 0xf)
        | if base_family == 0xf {
            (signature >> 12) & 0xf0
        } else {
            0
        };
    let ext = super::cpuid(0x8000_0001, 0).ok_or(PmuError::Unsupported)?;
    let has_extended_core = ext.ecx & EXTENDED_CORE != 0;
    let v2 = super::cpuid(0x8000_0022, 0).filter(|leaf| leaf.eax & 1 != 0);
    let has_global_control = v2.is_some();
    let count = v2.map_or(if has_extended_core { 6 } else { 4 }, |leaf| {
        (leaf.ebx & 0xf) as u8
    });
    let has_extended_core = has_extended_core || has_global_control;
    Ok(Caps {
        family,
        model,
        count,
        has_extended_core,
        has_global_control,
    })
}

impl Caps {
    pub(super) fn summary(&self) -> PmuCaps {
        let supported = self.count != 0 && self.has_event_encodings();
        PmuCaps {
            backend: BackendKind::AmdCore,
            counters: usize::from(self.count),
            width_bits: if self.count != 0 { COUNTER_WIDTH } else { 0 },
            supports_cycles: supported,
            supports_instructions: supported,
        }
    }

    fn has_event_encodings(&self) -> bool {
        // AMD OSRR 56255 and the Zen core event maps in Linux
        // arch/x86/events/amd/core.c define 76h/C0h on Zen families.
        // Neither event has a core-PMC index restriction in these profiles.
        // Unknown profiles still expose their resource inventory, but do not
        // inherit event semantics from a different family/model.
        matches!((self.family, self.model), (0x17 | 0x19 | 0x1a, 0x00..=0xff))
    }

    fn event_encoding(&self, event: Event) -> Result<(u64, u64)> {
        if !self.has_event_encodings() {
            return Err(PmuError::UnsupportedEvent);
        }
        let code = match event {
            Event::CpuCycles => 0x76,
            Event::Instructions => 0xc0,
        };
        Ok((code, (1u64 << self.count) - 1))
    }
}

pub(super) fn allocate(config: EventConfig, caps: &Caps, used: &[Option<usize>]) -> Result<Token> {
    let (encoding, allowed) = caps.event_encoding(config.event())?;
    for index in 0..u32::from(caps.count) {
        if allowed & (1 << index) == 0 || used.contains(&Some(index as usize)) {
            continue;
        }
        let (control_msr, counter_msr) = if caps.has_extended_core {
            (0xc001_0200 + index * 2, 0xc001_0201 + index * 2)
        } else {
            (0xc001_0000 + index, 0xc001_0004 + index)
        };
        // SAFETY: The current CPU advertises this counter and its register layout.
        if unsafe { rdmsr(control_msr) } != 0 {
            continue;
        }
        return Ok(Token {
            kind: Kind::AmdCore,
            index,
            width_bits: COUNTER_WIDTH,
            control_msr,
            counter_msr,
            global_msr: caps.has_global_control.then_some(GLOBAL_CONTROL),
            global_bit: 1 << index,
            saved_gate: 0,
            config_bits: encoding
                | (u64::from(config.is_user_counted()) << 16)
                | (u64::from(config.is_kernel_counted()) << 17),
            overflow_msr: caps.has_global_control.then_some(OVERFLOW_CONTROL),
        });
    }
    Err(PmuError::NoCounter)
}

#[cfg(any(test, ktest))]
mod tests {
    #[cfg(ktest)]
    use ostd_macros::ktest;

    use super::*;

    #[cfg_attr(ktest, ktest)]
    #[cfg_attr(not(ktest), test)]
    fn detected_counters_do_not_advertise_events_without_amd_encodings() {
        let caps = Caps {
            family: 0x15,
            model: 2,
            count: 6,
            has_extended_core: true,
            has_global_control: false,
        }
        .summary();

        assert_ne!(caps.counters(), 0);
        assert!(!caps.supports_event(Event::CpuCycles));
        assert!(!caps.supports_event(Event::Instructions));
    }
}
