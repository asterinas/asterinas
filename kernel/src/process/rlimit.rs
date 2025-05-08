// SPDX-License-Identifier: MPL-2.0
// FIXME: The resource limits should be respected by the corresponding subsystems of the kernel.

#![expect(non_camel_case_types)]

use core::{
    array,
    sync::atomic::{AtomicU64, Ordering},
};

use super::process_vm::{INIT_STACK_SIZE, USER_HEAP_SIZE_LIMIT};
use crate::prelude::*;

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

#[derive(Clone)]
pub struct ResourceLimits {
    rlimits: [RLimit64; RLIMIT_COUNT],
}

impl ResourceLimits {
    // Get a reference to a specific resource limit
    pub fn get_rlimit(&self, resource: ResourceType) -> &RLimit64 {
        &self.rlimits[resource as usize]
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        let mut rlimits: [RLimit64; RLIMIT_COUNT] = array::from_fn(|_| RLimit64::default());

        // Setting the resource limits with predefined values
        rlimits[ResourceType::RLIMIT_CPU as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_FSIZE as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_DATA as usize] =
            RLimit64::new(USER_HEAP_SIZE_LIMIT as u64, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_STACK as usize] =
            RLimit64::new(INIT_STACK_SIZE as u64, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_CORE as usize] = RLimit64::new(0, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_RSS as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_NPROC as usize] =
            RLimit64::new(INIT_RLIMIT_NPROC, INIT_RLIMIT_NPROC).unwrap();
        rlimits[ResourceType::RLIMIT_NOFILE as usize] =
            RLimit64::new(INIT_RLIMIT_NOFILE_CUR, INIT_RLIMIT_NOFILE_MAX).unwrap();
        rlimits[ResourceType::RLIMIT_MEMLOCK as usize] =
            RLimit64::new(INIT_RLIMIT_MEMLOCK, INIT_RLIMIT_MEMLOCK).unwrap();
        rlimits[ResourceType::RLIMIT_AS as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_LOCKS as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();
        rlimits[ResourceType::RLIMIT_SIGPENDING as usize] =
            RLimit64::new(INIT_RLIMIT_SIGPENDING, INIT_RLIMIT_SIGPENDING).unwrap();
        rlimits[ResourceType::RLIMIT_MSGQUEUE as usize] =
            RLimit64::new(INIT_RLIMIT_MSGQUEUE, INIT_RLIMIT_MSGQUEUE).unwrap();
        rlimits[ResourceType::RLIMIT_NICE as usize] =
            RLimit64::new(INIT_RLIMIT_NICE, INIT_RLIMIT_NICE).unwrap();
        rlimits[ResourceType::RLIMIT_RTPRIO as usize] =
            RLimit64::new(INIT_RLIMIT_RTPRIO, INIT_RLIMIT_RTPRIO).unwrap();
        rlimits[ResourceType::RLIMIT_RTTIME as usize] =
            RLimit64::new(RLIM_INFINITY, RLIM_INFINITY).unwrap();

        ResourceLimits { rlimits }
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
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

pub const RLIMIT_COUNT: usize = 16;

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct RawRLimit64 {
    pub cur: u64,
    pub max: u64,
}

#[derive(Debug)]
#[repr(C)]
pub struct RLimit64 {
    cur: AtomicU64,
    max: AtomicU64,
    lock: SpinLock<()>,
}

impl RLimit64 {
    pub fn new(cur_: u64, max_: u64) -> Result<Self> {
        if cur_ > max_ {
            return_errno_with_message!(Errno::EINVAL, "invalid rlimit");
        }
        Ok(Self {
            cur: AtomicU64::new(cur_),
            max: AtomicU64::new(max_),
            lock: SpinLock::new(()),
        })
    }

    /// Gets the current rlimit without synchronization.
    pub fn get_cur(&self) -> u64 {
        self.cur.load(Ordering::Relaxed)
    }

    /// Gets the max rlimit without synchronization.
    pub fn get_max(&self) -> u64 {
        self.max.load(Ordering::Relaxed)
    }

    /// Gets the rlimit with synchronization.
    ///
    /// Only called when handling the `getrlimit` or `prlimit` syscall.
    pub fn get_cur_and_max(&self) -> (u64, u64) {
        let _guard = self.lock.lock();
        (self.get_cur(), self.get_max())
    }

    /// Sets the rlimit with synchronization.
    ///
    /// Only called when handling the `setrlimit` or `prlimit` syscall.
    pub fn set_cur_and_max(&self, new_cur: u64, new_max: u64) -> Result<()> {
        if new_cur > new_max {
            return_errno_with_message!(Errno::EINVAL, "invalid rlimit");
        }
        let _guard = self.lock.lock();
        self.cur.store(new_cur, Ordering::Relaxed);
        self.max.store(new_max, Ordering::Relaxed);
        Ok(())
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
        let (cur, max) = self.get_cur_and_max();
        Self {
            cur: AtomicU64::new(cur),
            max: AtomicU64::new(max),
            lock: SpinLock::new(()),
        }
    }
}
