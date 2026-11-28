// SPDX-License-Identifier: MPL-2.0

//! Intel architectural PMU discovery and single-event allocation.
//! See Intel SDM, Architectural Performance Monitoring and CPUID leaf 0AH.
//!
//! This backend implements counting for architectural PMU versions 1 through 6:
//! - Version 1 uses general-purpose counters without global control registers.
//! - Versions 2 through 4 enumerate fixed counters through EDX.
//! - Version 5 additionally uses the fixed-counter support bitmap in ECX.
//! - Version 6 uses CPUID.23H true-view bitmaps and the 19xxH MSR aliases
//!   when the extended counter sub-leaf is enumerated on this logical CPU.
//!
//! Extended capabilities are cached in full, including UMASK2, EQ, ACR, and
//! architectural PEBS enumeration. These two counting events leave UMASK2 and
//! EQ clear and do not enable ACR, PEBS, or interrupts.
//! See Intel SDM vol. 3B, sections 22.2.6, 22.2.7, 22.2.9, and 22.11:
//! <https://cdrdv2-public.intel.com/874250/253669-090-sdm-vol-3b.pdf>.
//! Version 0 and versions beyond 6 remain unsupported.

use alloc::vec::Vec;
use core::arch::x86_64::CpuidResult;

use x86::msr::rdmsr;

use super::counter::{Kind, Token};
use crate::pmu::{BackendKind, Event, EventConfig, PmuCaps, PmuError, Result};

const FIXED_CONTROL: u32 = 0x38d;
const GLOBAL_CONTROL: u32 = 0x38f;
const OVERFLOW_CONTROL: u32 = 0x390;
const FIXED_BASE: u32 = 0x309;
const EVENT_SELECT_BASE: u32 = 0x186;
const PMC_BASE: u32 = 0xc1;
const V6_GP_BASE: u32 = 0x1900;
const V6_FIXED_BASE: u32 = 0x1980;
const V6_COUNTER_STRIDE: u32 = 4;
const ARCH_PERFMON_EXT: u32 = 1 << 8;

#[derive(Debug)]
pub(crate) struct Caps {
    version: u8,
    gp_count: u8,
    gp_mask: u32,
    gp_width: u8,
    fixed_mask: u32,
    fixed_width: u8,
    event_mask: u32,
    extended: ExtendedCaps,
}

pub(super) fn probe() -> Result<Caps> {
    let leaf = super::cpuid(0xa, 0).ok_or(PmuError::Unsupported)?;
    let version = leaf.eax as u8;
    if !(1..=6).contains(&version) {
        return Err(PmuError::Unsupported);
    }
    let mut gp_count = (leaf.eax >> 8) as u8;
    let gp_width = (leaf.eax >> 16) as u8;
    let event_length = (leaf.eax >> 24).min(32);
    let mut event_mask = !leaf.ebx & low_mask(event_length);
    let (mut fixed_mask, fixed_width) = if version >= 2 {
        // Version 5 adds individually enumerated fixed counters.
        (
            low_mask(leaf.edx & 0x1f) | if version >= 5 { leaf.ecx } else { 0 },
            ((leaf.edx >> 5) & 0xff) as u8,
        )
    } else {
        (0, 0)
    };
    let extended = if version == 6 {
        ExtendedCaps::probe()?
    } else {
        ExtendedCaps { leaves: Vec::new() }
    };
    let gp_mask = if let Some(counters) = extended.subleaf(1) {
        // Leaf 23H is the actual per-logical-CPU view, not an addition to 0AH.
        gp_count = counters.eax.count_ones() as u8;
        fixed_mask = counters.ebx;
        if fixed_mask & !low_mask(16) != 0 {
            return Err(PmuError::HardwareFault);
        }
        counters.eax
    } else {
        low_mask(u32::from(gp_count))
    };
    if let Some(events) = extended.subleaf(3) {
        // Unlike leaf 0AH EBX, a set bit here means the event is supported.
        event_mask = events.eax;
    }
    if (gp_count != 0 && !(1..=64).contains(&gp_width))
        || (fixed_mask != 0 && !(1..=64).contains(&fixed_width))
    {
        return Err(PmuError::HardwareFault);
    }
    Ok(Caps {
        version,
        gp_count,
        gp_mask,
        gp_width,
        fixed_mask,
        fixed_width,
        event_mask,
        extended,
    })
}

pub(super) fn allocate(config: EventConfig, caps: &Caps, used: &[Option<usize>]) -> Result<Token> {
    let (fixed, event_bit, event_code) = event_encoding(config.event());
    let use_aliases = caps.extended.subleaf(1).is_some();
    let fixed_counter_msr = if use_aliases {
        V6_FIXED_BASE + V6_COUNTER_STRIDE * fixed
    } else {
        FIXED_BASE + fixed
    };
    let has_fixed = caps.fixed_mask & (1 << fixed) != 0;
    if has_fixed && !used.contains(&Some(256 + fixed as usize)) {
        let shift = fixed * 4;
        // SAFETY: The current CPU enumerates this fixed counter.
        if unsafe { rdmsr(FIXED_CONTROL) } & (0xf << shift) == 0
            && caps.extended.is_counter_idle(fixed, Kind::IntelFixed)
        {
            return Ok(Token {
                kind: Kind::IntelFixed,
                index: fixed,
                width_bits: caps.fixed_width,
                control_msr: FIXED_CONTROL,
                counter_msr: fixed_counter_msr,
                global_msr: Some(GLOBAL_CONTROL),
                global_bit: 1 << (32 + fixed),
                saved_gate: 0,
                config_bits: (u64::from(config.is_kernel_counted())
                    | (u64::from(config.is_user_counted()) << 1))
                    << shift,
                overflow_msr: Some(OVERFLOW_CONTROL),
            });
        }
    }
    if caps.event_mask & (1 << event_bit) == 0 || caps.gp_count == 0 {
        return Err(if has_fixed {
            PmuError::NoCounter
        } else {
            PmuError::UnsupportedEvent
        });
    }
    // Iterate the actual bitmap: V6 counters need not have contiguous indices.
    for index in 0..32 {
        if caps.gp_mask & (1 << index) == 0 || used.contains(&Some(index as usize)) {
            continue;
        }
        let (counter_msr, control_msr) = if use_aliases {
            let base = V6_GP_BASE + V6_COUNTER_STRIDE * index;
            (base, base + 1)
        } else {
            (PMC_BASE + index, EVENT_SELECT_BASE + index)
        };
        // SAFETY: The current CPU enumerates this general-purpose counter and its layout.
        if unsafe { rdmsr(control_msr) } != 0
            || !caps.extended.is_counter_idle(index, Kind::IntelGeneral)
        {
            continue;
        }
        return Ok(Token {
            kind: Kind::IntelGeneral,
            index,
            width_bits: caps.gp_width,
            control_msr,
            counter_msr,
            global_msr: (caps.version >= 2).then_some(GLOBAL_CONTROL),
            global_bit: 1 << index,
            saved_gate: 0,
            config_bits: event_code
                | (u64::from(config.is_user_counted()) << 16)
                | (u64::from(config.is_kernel_counted()) << 17),
            overflow_msr: (caps.version >= 2).then_some(OVERFLOW_CONTROL),
        });
    }
    Err(PmuError::NoCounter)
}

impl Caps {
    pub(super) fn summary(&self) -> PmuCaps {
        PmuCaps {
            backend: BackendKind::IntelArchitectural,
            counters: usize::from(self.gp_count) + self.fixed_mask.count_ones() as usize,
            width_bits: match (self.gp_count != 0, self.fixed_mask != 0) {
                (true, true) => self.gp_width.min(self.fixed_width),
                (true, false) => self.gp_width,
                (false, true) => self.fixed_width,
                _ => 0,
            },
            supports_cycles: self.supports_event(Event::CpuCycles),
            supports_instructions: self.supports_event(Event::Instructions),
        }
    }
    fn supports_event(&self, event: Event) -> bool {
        let (fixed, event_bit, _) = event_encoding(event);
        self.fixed_mask & (1 << fixed) != 0
            || self.gp_mask != 0 && self.event_mask & (1 << event_bit) != 0
    }
}

fn event_encoding(event: Event) -> (u32, u32, u64) {
    match event {
        Event::CpuCycles => (1, 0, 0x3c),
        Event::Instructions => (0, 1, 0xc0),
    }
}

/// Raw snapshots of all advertised extended sub-leaves on this logical CPU.
///
/// Sub-leaf 0 enumerates features (UMASK2/EQ), valid sub-leaves and TMA slots;
/// 1 enumerates counters, 2 ACR, 3 events, and 4/5 architectural PEBS.
/// Keeping every advertised leaf avoids truncating discovery to our two events.
#[derive(Debug)]
struct ExtendedCaps {
    leaves: Vec<(u32, CpuidResult)>,
}

impl ExtendedCaps {
    fn probe() -> Result<Self> {
        let mut caps = Self { leaves: Vec::new() };
        let has_extended = super::cpuid(7, 0).is_some_and(|leaf| leaf.eax >= 1)
            && super::cpuid(7, 1).is_some_and(|leaf| leaf.eax & ARCH_PERFMON_EXT != 0);
        if !has_extended {
            return Ok(caps);
        }
        let root = super::cpuid(0x23, 0).ok_or(PmuError::HardwareFault)?;
        let supported = root.eax;
        // The root is always saved, even if its own bit is absent.
        caps.leaves
            .try_reserve_exact(1 + (supported >> 1).count_ones() as usize)
            .map_err(|_| PmuError::NoCounter)?;
        caps.leaves.push((0, root));
        for index in 1..32 {
            if supported & (1 << index) != 0 {
                let leaf = super::cpuid(0x23, index).ok_or(PmuError::HardwareFault)?;
                caps.leaves.push((index, leaf));
            }
        }
        Ok(caps)
    }

    fn subleaf(&self, index: u32) -> Option<&CpuidResult> {
        self.leaves
            .iter()
            .find(|(id, _)| *id == index)
            .map(|(_, leaf)| leaf)
    }

    fn is_counter_idle(&self, index: u32, kind: Kind) -> bool {
        let is_fixed = kind == Kind::IntelFixed;
        let counter_msr =
            if is_fixed { V6_FIXED_BASE } else { V6_GP_BASE } + V6_COUNTER_STRIDE * index;
        let acr = self.subleaf(2).is_some_and(|leaf| {
            let mask = if is_fixed { leaf.ebx } else { leaf.eax };
            mask & (1 << index) != 0
        });
        let pebs = self.subleaf(5).is_some_and(|leaf| {
            let mask = if is_fixed { leaf.ecx } else { leaf.eax };
            mask & (1 << index) != 0
        });
        // SAFETY: These optional registers are accessed only when the current CPU
        // enumerates them for this counter. Nonzero values mean it is already in use.
        unsafe {
            (!acr || rdmsr(counter_msr + 2) == 0) && (!(acr || pebs) || rdmsr(counter_msr + 3) == 0)
        }
    }
}

fn low_mask(bits: u32) -> u32 {
    if bits >= 32 {
        u32::MAX
    } else {
        (1 << bits) - 1
    }
}

#[cfg(ktest)]
mod tests {
    use ostd_macros::ktest;

    use super::*;

    #[ktest]
    fn fixed_instruction_counter_does_not_advertise_absent_cycles() {
        let caps = Caps {
            version: 2,
            gp_count: 0,
            gp_mask: 0,
            gp_width: 0,
            fixed_mask: 1,
            fixed_width: 48,
            event_mask: 0,
            extended: ExtendedCaps { leaves: Vec::new() },
        }
        .summary();

        assert_eq!(caps.counters(), 1);
        assert!(caps.supports_event(Event::Instructions));
        assert!(!caps.supports_event(Event::CpuCycles));
    }
}
