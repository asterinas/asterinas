// SPDX-License-Identifier: MPL-2.0

//! Address space size and RSS counting/limiting.

use core::sync::atomic::Ordering;

use ostd::{
    cpu::{CpuId, PinCurrentCpu, all_cpus, num_cpus},
    task::DisabledPreemptGuard,
};

use super::Vmar;
use crate::{
    prelude::*,
    process::{Process, ResourceType, rlimit::RLIM_INFINITY},
};

impl Vmar {
    /// Adds the mapping size.
    ///
    /// Returns `Err` if the new size exceeds [`ResourceType::RLIMIT_AS`].
    pub(super) fn add_mapping_size(
        &self,
        preempt_guard: &DisabledPreemptGuard,
        add_size: usize,
    ) -> Result<()> {
        let add_size = add_size as isize;
        let cur_cpu = preempt_guard.current_cpu();

        let rlimit_as = get_rlimit_as();

        // First, try adding directly to this CPU if the quota is sufficient.

        let rlimit_this_cpu = rlimit_as_on_cpu(rlimit_as, cur_cpu);

        if self
            .mapped_vm_size
            .get_on_cpu(cur_cpu)
            .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                if let Some(new) = old.checked_add(add_size)
                    && new <= rlimit_this_cpu
                {
                    Some(new)
                } else {
                    None
                }
            })
            .is_ok()
        {
            return Ok(());
        };

        // Otherwise, add to all CPUs.

        let mut remaining = add_size;
        for cpu in all_cpus() {
            let mut crammed = 0;
            let quota = rlimit_as_on_cpu(rlimit_as, cpu);

            if self
                .mapped_vm_size
                .get_on_cpu(cur_cpu)
                .fetch_update(Ordering::Release, Ordering::Acquire, |old| {
                    let new = old + remaining;
                    if new < quota {
                        crammed = remaining;
                        Some(new)
                    } else {
                        crammed = quota - old;
                        Some(quota as isize)
                    }
                })
                .is_ok()
                && remaining == crammed
            {
                return Ok(());
            };

            remaining -= crammed;
        }

        // We failed. Revert the added values and return error.
        debug_assert!(remaining > 0);
        self.mapped_vm_size
            .get_on_cpu(cur_cpu)
            .fetch_sub(add_size - remaining, Ordering::AcqRel);

        Err(Error::with_message(
            Errno::ENOMEM,
            "the new mapping size exceeds the limit",
        ))
    }
}

/// The type representing categories of Resident Set Size (RSS).
///
/// See <https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/linux/mm_types_task.h#L26..L32>
#[repr(u32)]
#[expect(non_camel_case_types)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum RssType {
    RSS_FILEPAGES = 0,
    RSS_ANONPAGES = 1,
}

pub(super) const NUM_RSS_COUNTERS: usize = 2;

/// A helper struct to track resident set and address space size changes.
pub struct RsAsDelta<'a> {
    rs_as_delta: [isize; NUM_RSS_COUNTERS],
    as_delta: isize,
    operated_vmar: &'a Vmar,
}

impl<'a> RsAsDelta<'a> {
    pub fn new(operated_vmar: &'a Vmar) -> Self {
        Self {
            rs_as_delta: [0; NUM_RSS_COUNTERS],
            as_delta: 0,
            operated_vmar,
        }
    }

    pub fn add_rs(&mut self, rss_type: RssType, increment: isize) {
        self.rs_as_delta[rss_type as usize] += increment;
    }

    pub fn sub_as(&mut self, decrement: usize) {
        self.as_delta -= decrement as isize;
    }
}

impl Drop for RsAsDelta<'_> {
    fn drop(&mut self) {
        for i in 0..NUM_RSS_COUNTERS {
            let rss_type = RssType::try_from(i as u32).unwrap();
            let delta = self.rs_as_delta[rss_type as usize];
            self.operated_vmar.add_rss_counter(rss_type, delta);
        }
        // `current_racy` is OK because subtracting on any CPUs is OK.
        debug_assert!(self.as_delta <= 0);
        self.operated_vmar
            .mapped_vm_size
            .get_on_cpu(CpuId::current_racy())
            .fetch_add(self.as_delta, Ordering::AcqRel);
    }
}

fn get_rlimit_as() -> u64 {
    let Some(process) = Process::current() else {
        // When building a `Process`, the kernel task needs to build
        // some `VmMapping`s, in which case this branch is reachable.
        return RLIM_INFINITY;
    };

    process
        .resource_limits()
        .get_rlimit(ResourceType::RLIMIT_AS)
        .get_cur()
}

const RLIMIT_AS_PER_CPU_INFINITY: isize = isize::MAX;

fn rlimit_as_on_cpu(total_rlimit: u64, cpu: CpuId) -> isize {
    if total_rlimit == RLIM_INFINITY || total_rlimit > RLIMIT_AS_PER_CPU_INFINITY as u64 {
        return RLIMIT_AS_PER_CPU_INFINITY;
    }

    let num_cpus = num_cpus() as u64;
    let val = if cpu == CpuId::bsp() {
        total_rlimit / num_cpus + total_rlimit % num_cpus
    } else {
        total_rlimit / num_cpus
    };

    val as isize
}
