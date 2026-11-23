// SPDX-License-Identifier: MPL-2.0

//! Linux counting attributes, including versioned user-memory decoding.

use ostd::{
    const_assert,
    cpu::CpuId,
    mm::VmIo,
    pmu::{CountingScope, Event, EventConfig},
};

use crate::{
    events::perf::{EventSpec, ReadFormat},
    prelude::*,
};

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_ATTR_SIZE_VER0: u32 = 64;

/// Reads and validates a counting attribute before attaching to the target.
pub(super) fn read_counting(
    addr: Vaddr,
    ctx: &Context,
    cpu: Option<CpuId>,
) -> Result<(EventSpec, ReadFormat)> {
    let user_space = ctx.user_space();
    copy_attr(addr, &user_space)?.parse_counting(cpu)
}

const ATTR_DISABLED: u64 = 1 << 0;
const ATTR_EXCLUDE_USER: u64 = 1 << 4;
const ATTR_EXCLUDE_KERNEL: u64 = 1 << 5;
const ATTR_ENABLE_ON_EXEC: u64 = 1 << 12;

const ATTR_EXCLUDE_HV: u64 = 1 << 6;
const ATTR_EXCLUDE_IDLE: u64 = 1 << 7;
const ATTR_EXCLUDE_GUEST: u64 = 1 << 20;

// Per-thread counting never includes the idle task or a guest VM. S-mode
// excludes firmware M-mode. There is no supported hypervisor execution mode.
const SUPPORTED: u64 = ATTR_DISABLED
    | ATTR_EXCLUDE_USER
    | ATTR_EXCLUDE_KERNEL
    | ATTR_EXCLUDE_HV
    | ATTR_EXCLUDE_IDLE
    | ATTR_ENABLE_ON_EXEC
    | ATTR_EXCLUDE_GUEST;

const fn supported_flags(flags: u64) -> bool {
    flags & !SUPPORTED == 0
}

/// Accepts counting metadata used by standard Linux `perf stat`.
///
/// `PERF_SAMPLE_IDENTIFIER` describes a sample layout but does not request
/// sampling when the period is zero.
/// All actual sampling remains unsupported.
/// See Linux `tools/perf/builtin-stat.c`, `create_perf_stat_counter`.
const fn supported_sampling(sample_period: u64, sample_type: u64) -> bool {
    const PERF_SAMPLE_IDENTIFIER: u64 = 1 << 16;
    sample_period == 0 && sample_type & !PERF_SAMPLE_IDENTIFIER == 0
}

// Reference: Linux `include/uapi/linux/perf_event.h` (`PERF_ATTR_SIZE_VER0`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct PerfEventAttrV0 {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
}

impl PerfEventAttrV0 {
    fn parse_counting(&self, cpu: Option<CpuId>) -> Result<(EventSpec, ReadFormat)> {
        let event = match (self.type_, self.config) {
            (PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES) => Event::CpuCycles,
            (PERF_TYPE_HARDWARE, PERF_COUNT_HW_INSTRUCTIONS) => Event::Instructions,
            _ => {
                return Err(errno!(
                    EOPNOTSUPP,
                    "only hardware cycles and instructions are supported"
                ));
            }
        };
        if !supported_flags(self.flags)
            || !supported_sampling(self.sample_period, self.sample_type)
            || self.wakeup_events != 0
            || self.bp_type != 0
            || self.config1 != 0
        {
            return Err(errno!(EOPNOTSUPP, "unsupported perf counting attributes"));
        }
        let read_format = ReadFormat::from_bits(self.read_format)
            .ok_or_else(|| errno!(EOPNOTSUPP, "unsupported perf read format"))?;
        let is_user_counted = self.flags & ATTR_EXCLUDE_USER == 0;
        let is_kernel_counted = self.flags & ATTR_EXCLUDE_KERNEL == 0;
        let scope = match (is_user_counted, is_kernel_counted) {
            (true, false) => CountingScope::User,
            (false, true) => CountingScope::Kernel,
            (true, true) => CountingScope::UserAndKernel,
            (false, false) => {
                return Err(errno!(
                    EINVAL,
                    "all requested privilege levels are excluded"
                ));
            }
        };
        Ok((
            EventSpec {
                config: EventConfig::new(event, scope),
                cpu,
                is_initially_disabled: self.flags & ATTR_DISABLED != 0,
                should_enable_on_exec: self.flags & ATTR_ENABLE_ON_EXEC != 0,
            },
            read_format,
        ))
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct AttrHeader {
    type_: u32,
    size: u32,
}

const ATTR_FLAGS_OFFSET: usize = 40;
const ATTR_CONFIG1_OFFSET: usize = 56;

const_assert!(size_of::<PerfEventAttrV0>() == PERF_ATTR_SIZE_VER0 as usize);
const_assert!(core::mem::offset_of!(PerfEventAttrV0, flags) == ATTR_FLAGS_OFFSET);
const_assert!(core::mem::offset_of!(PerfEventAttrV0, config1) == ATTR_CONFIG1_OFFSET);

fn copy_attr(addr: Vaddr, user_space: &CurrentUserSpace<'_>) -> Result<PerfEventAttrV0> {
    let header: AttrHeader = user_space.read_val(addr)?;
    let size = if header.size == 0 {
        PERF_ATTR_SIZE_VER0 as usize
    } else {
        header.size as usize
    };

    if !(PERF_ATTR_SIZE_VER0 as usize..=PAGE_SIZE).contains(&size) {
        return report_attr_size(addr, user_space);
    }

    let attr: PerfEventAttrV0 = user_space.read_val(addr)?;
    if attr.size != header.size {
        return Err(errno!(EINVAL, "the attribute size changed while copying"));
    }

    if size > PERF_ATTR_SIZE_VER0 as usize {
        let tail_addr = addr
            .checked_add(PERF_ATTR_SIZE_VER0 as usize)
            .ok_or_else(|| errno!(EFAULT, "attribute address overflow"))?;
        let end = addr
            .checked_add(size)
            .ok_or_else(|| errno!(EFAULT, "attribute address overflow"))?;
        // Read the entire extension before reporting its contents, so a fault
        // has the same priority as for the fixed prefix.
        let mut buf = [0u8; 64];
        let mut has_nonzero_byte = false;
        for offset in (tail_addr..end).step_by(buf.len()) {
            let chunk_len = (end - offset).min(buf.len());
            let chunk = &mut buf[..chunk_len];
            user_space.read_bytes(offset, chunk)?;
            has_nonzero_byte |= chunk.iter().any(|&byte| byte != 0);
        }
        if has_nonzero_byte {
            return report_attr_size(addr, user_space);
        }
    }
    Ok(attr)
}

fn report_attr_size(addr: Vaddr, user_space: &CurrentUserSpace<'_>) -> Result<PerfEventAttrV0> {
    let size_addr = addr
        .checked_add(core::mem::offset_of!(PerfEventAttrV0, size))
        .ok_or_else(|| errno!(EFAULT, "attribute address overflow"))?;
    user_space.write_val(size_addr, &PERF_ATTR_SIZE_VER0)?;
    Err(errno!(
        E2BIG,
        "unsupported perf_event_attr size or extension"
    ))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn standard_perf_stat_identifier_does_not_request_sampling() {
        assert!(supported_sampling(0, 1 << 16));
        assert!(supported_sampling(0, 0));
        assert!(!supported_sampling(1, 1 << 16));
        assert!(!supported_sampling(0, (1 << 16) | 1));
    }

    #[ktest]
    fn inheritance_is_rejected_even_with_enable_on_exec() {
        assert!(!supported_flags(1 << 1));
        assert!(!supported_flags(
            (1 << 1) | ATTR_DISABLED | ATTR_ENABLE_ON_EXEC
        ));
    }

    #[ktest]
    fn unsupported_modes_are_not_silently_accepted() {
        assert!(!supported_flags(1 << 63)); // reserved
        assert!(!supported_flags(1 << 2)); // pinned
        assert!(!supported_flags(1 << 19)); // exclude_host
    }

    #[ktest]
    fn no_inherit_perf_stat_flags_are_supported() {
        assert!(supported_flags(
            ATTR_DISABLED | ATTR_ENABLE_ON_EXEC | ATTR_EXCLUDE_KERNEL | ATTR_EXCLUDE_GUEST
        ));
        assert!(supported_flags(0));
    }
}
