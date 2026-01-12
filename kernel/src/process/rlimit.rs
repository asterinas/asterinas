// SPDX-License-Identifier: MPL-2.0
// FIXME: The resource limits should be respected by the corresponding subsystems of the kernel.

use core::{
    array,
    sync::atomic::{AtomicU64, Ordering},
};

use super::process_vm::INIT_STACK_SIZE;
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet},
};

// Constants for the boot-time rlimit defaults
// See https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/asm-generic/resource.h#L11
const RLIM_INFINITY: u64 = u64::MAX;
const INIT_RLIMIT_NPROC: u64 = 0;
const INIT_RLIMIT_NICE: u64 = 0;
const INIT_RLIMIT_SIGPENDING: u64 = 0;
const INIT_RLIMIT_RTPRIO: u64 = 0;
// https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/uapi/linux/fs.h#L37
const INIT_RLIMIT_NOFILE_CUR: u64 = 1024;
const INIT_RLIMIT_NOFILE_MAX: u64 = 4096;
// https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/uapi/linux/resource.h#L79
const INIT_RLIMIT_MEMLOCK: u64 = 8 * 1024 * 1024;
// https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/uapi/linux/mqueue.h#L26
const INIT_RLIMIT_MSGQUEUE: u64 = 819200;
// https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/fs/file.c#L90
pub const SYSCTL_NR_OPEN: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct ResourceLimits {
    rlimits: [RLimit64; RLIMIT_COUNT],
}

impl ResourceLimits {
    /// Returns a reference to a specific resource limit.
    pub fn get_rlimit(&self, resource: ResourceType) -> &RLimit64 {
        &self.rlimits[resource as usize]
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        let mut rlimits: [RLimit64; RLIMIT_COUNT] = array::from_fn(|_| RLimit64::default());

        // Sets the resource limits with predefined values
        rlimits[ResourceType::RLIMIT_CPU as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_FSIZE as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_DATA as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_STACK as usize] =
            RLimit64::new(INIT_STACK_SIZE as u64, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_CORE as usize] = RLimit64::new(0, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_RSS as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_NPROC as usize] =
            RLimit64::new(INIT_RLIMIT_NPROC, INIT_RLIMIT_NPROC);
        rlimits[ResourceType::RLIMIT_NOFILE as usize] =
            RLimit64::new(INIT_RLIMIT_NOFILE_CUR, INIT_RLIMIT_NOFILE_MAX);
        rlimits[ResourceType::RLIMIT_MEMLOCK as usize] =
            RLimit64::new(INIT_RLIMIT_MEMLOCK, INIT_RLIMIT_MEMLOCK);
        rlimits[ResourceType::RLIMIT_AS as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_LOCKS as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_SIGPENDING as usize] =
            RLimit64::new(INIT_RLIMIT_SIGPENDING, INIT_RLIMIT_SIGPENDING);
        rlimits[ResourceType::RLIMIT_MSGQUEUE as usize] =
            RLimit64::new(INIT_RLIMIT_MSGQUEUE, INIT_RLIMIT_MSGQUEUE);
        rlimits[ResourceType::RLIMIT_NICE as usize] =
            RLimit64::new(INIT_RLIMIT_NICE, INIT_RLIMIT_NICE);
        rlimits[ResourceType::RLIMIT_RTPRIO as usize] =
            RLimit64::new(INIT_RLIMIT_RTPRIO, INIT_RLIMIT_RTPRIO);
        rlimits[ResourceType::RLIMIT_RTTIME as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);

        ResourceLimits { rlimits }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromInt)]
#[expect(non_camel_case_types)]
pub enum ResourceType {
    RLIMIT_CPU = 0,
    RLIMIT_FSIZE = 1,
    RLIMIT_DATA = 2,
    RLIMIT_STACK = 3,
    RLIMIT_CORE = 4,
    RLIMIT_RSS = 5,
    RLIMIT_NPROC = 6,
    RLIMIT_NOFILE = 7,
    RLIMIT_MEMLOCK = 8,
    RLIMIT_AS = 9,
    RLIMIT_LOCKS = 10,
    RLIMIT_SIGPENDING = 11,
    RLIMIT_MSGQUEUE = 12,
    RLIMIT_NICE = 13,
    RLIMIT_RTPRIO = 14,
    RLIMIT_RTTIME = 15,
}

const RLIMIT_COUNT: usize = 16;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct RawRLimit64 {
    pub cur: u64,
    pub max: u64,
}

#[derive(Debug)]
pub struct RLimit64 {
    cur: AtomicU64,
    max: AtomicU64,
    lock: SpinLock<()>,
}

impl RLimit64 {
    #[track_caller]
    pub(self) const fn new(cur: u64, max: u64) -> Self {
        assert!(cur <= max, "the current rlimit exceeds the max rlimit");
        Self {
            cur: AtomicU64::new(cur),
            max: AtomicU64::new(max),
            lock: SpinLock::new(()),
        }
    }

    /// Returns the current rlimit without synchronization.
    pub fn get_cur(&self) -> u64 {
        self.cur.load(Ordering::Relaxed)
    }

    /// Returns the max rlimit without synchronization.
    fn get_max(&self) -> u64 {
        self.max.load(Ordering::Relaxed)
    }

    /// Gets the rlimit with synchronization.
    ///
    /// Only called when handling the `getrlimit` or `prlimit` syscall.
    pub fn get_raw_rlimit(&self) -> RawRLimit64 {
        let _guard = self.lock.lock();
        RawRLimit64 {
            cur: self.cur.load(Ordering::Relaxed),
            max: self.max.load(Ordering::Relaxed),
        }
    }

    /// Sets the rlimit with synchronization and returns the old value.
    ///
    /// Only called when handling the `setrlimit` or `prlimit` syscall.
    pub fn set_raw_rlimit(&self, new: RawRLimit64, ctx: &Context) -> Result<RawRLimit64> {
        if new.cur > new.max {
            return_errno_with_message!(Errno::EINVAL, "the current rlimit exceeds the max rlimit");
        }
        let _guard = self.lock.lock();
        if new.max > self.get_max() {
            let init_user_ns = UserNamespace::get_init_singleton();
            init_user_ns.check_cap(CapSet::SYS_RESOURCE, ctx.posix_thread)?;
        }
        let old = RawRLimit64 {
            cur: self.cur.load(Ordering::Relaxed),
            max: self.max.load(Ordering::Relaxed),
        };
        self.set_raw_rlimit_unchecked(new);
        Ok(old)
    }

    /// Sets the rlimit _without_ synchronization and permission check.
    ///
    /// Only called during init process creation.
    #[track_caller]
    pub(self) fn set_raw_rlimit_unchecked(&self, new: RawRLimit64) {
        assert!(
            new.cur <= new.max,
            "the current rlimit exceeds the max rlimit"
        );
        self.cur.store(new.cur, Ordering::Relaxed);
        self.max.store(new.max, Ordering::Relaxed);
    }
}

impl Default for RLimit64 {
    fn default() -> Self {
        Self {
            cur: AtomicU64::new(RLIM_INFINITY),
            max: AtomicU64::new(RLIM_INFINITY),
            lock: SpinLock::new(()),
        }
    }
}

impl Clone for RLimit64 {
    fn clone(&self) -> Self {
        let raw_limit = self.get_raw_rlimit();
        Self {
            cur: AtomicU64::new(raw_limit.cur),
            max: AtomicU64::new(raw_limit.max),
            lock: SpinLock::new(()),
        }
    }
}

/// Creates resource limits for the init process.
///
/// This function should be used when creating the init process to ensure it has
/// appropriate resource limits.
pub(super) fn new_resource_limits_for_init() -> ResourceLimits {
    let resource_limits = ResourceLimits::default();
    // FIXME: The value should be calculated based on the system capacity. For now, we set a
    // fixed value. In Linux, this value is determined by the kernel based on the available memory
    // and other factors.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/kernel/fork.c#L761>
    let max_threads: u64 = 100000;
    let raw_rlimit = RawRLimit64 {
        cur: max_threads / 2,
        max: max_threads / 2,
    };
    resource_limits
        .get_rlimit(ResourceType::RLIMIT_NPROC)
        .set_raw_rlimit_unchecked(raw_rlimit);
    resource_limits
        .get_rlimit(ResourceType::RLIMIT_SIGPENDING)
        .set_raw_rlimit_unchecked(raw_rlimit);
    resource_limits
}
