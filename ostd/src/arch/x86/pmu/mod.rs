// SPDX-License-Identifier: MPL-2.0

//! x86-64 PMU backend for Intel architectural counters and AMD Zen core counters.
//!
//! Register layouts and event encodings follow:
//! - Intel 64 and IA-32 Architectures Software Developer's Manual, Volume 3B,
//!   Chapter 20 ("Performance Monitoring").
//! - AMD64 Architecture Programmer's Manual, Volume 2, Section 13.2
//!   ("Core Performance Counters").
//!
//! The backend relies on the CPU or hypervisor making the MSRs for its
//! advertised PMU capabilities accessible on the current logical CPU.

use x86_64::registers::control::{Cr4, Cr4Flags};

pub(crate) use self::counter::{Caps, Token};
use crate::{
    arch::cpu::cpuid::cpuid,
    pmu::{EventConfig, PmuError, Result},
};

mod amd;
mod counter;
mod intel;

pub(crate) const HAS_BACKEND: bool = true;

/// Probes the current x86-64 CPU for a supported hardware PMU interface.
pub(crate) fn probe() -> Result<Caps> {
    crate::arch::if_tdx_enabled!({
        return Err(PmuError::Unsupported);
    });

    // Reject platforms where user-space `rdpmc` is globally enabled in CR4.
    if Cr4::read().contains(Cr4Flags::PERFORMANCE_MONITOR_COUNTER) {
        return Err(PmuError::Unsupported);
    }

    let base_leaf = cpuid(0, 0).ok_or(PmuError::Unsupported)?;
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&base_leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&base_leaf.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&base_leaf.ecx.to_le_bytes());

    if &vendor == b"GenuineIntel" {
        return intel::probe().map(Caps::Intel);
    }
    if &vendor == b"AuthenticAMD" {
        return amd::probe().map(Caps::Amd);
    }

    Err(PmuError::Unsupported)
}

pub(crate) fn configure(
    config: EventConfig,
    caps: &Caps,
    used: &[Option<usize>],
) -> Result<(Token, u64)> {
    counter::configure(config, caps, used)
}
