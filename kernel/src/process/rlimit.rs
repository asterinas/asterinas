// SPDX-License-Identifier: MPL-2.0
// FIXME: The resource limits should be respected by the corresponding subsystems of the kernel.

#![allow(non_camel_case_types)]

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

pub struct ResourceLimits {
    rlimits: [RLimit64; RLIMIT_COUNT],
}

impl ResourceLimits {
    // Get a reference to a specific resource limit
    pub fn get_rlimit(&self, resource: ResourceType) -> &RLimit64 {
        &self.rlimits[resource as usize]
    }

    // Get a mutable reference to a specific resource limit
    pub fn get_rlimit_mut(&mut self, resource: ResourceType) -> &mut RLimit64 {
        &mut self.rlimits[resource as usize]
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        let mut rlimits = [RLimit64::default(); RLIMIT_COUNT];

        // Setting the resource limits with predefined values
        rlimits[ResourceType::RLIMIT_CPU as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_FSIZE as usize] = RLimit64::new(RLIM_INFINITY, RLIM_INFINITY);
        rlimits[ResourceType::RLIMIT_DATA as usize] =
            RLimit64::new(USER_HEAP_SIZE_LIMIT as u64, RLIM_INFINITY);
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
pub struct RLimit64 {
    cur: u64,
    max: u64,
}

impl RLimit64 {
    pub fn new(cur: u64, max: u64) -> Self {
        Self { cur, max }
    }

    pub fn get_cur(&self) -> u64 {
        self.cur
    }

    pub fn get_max(&self) -> u64 {
        self.max
    }

    pub fn is_valid(&self) -> bool {
        self.cur <= self.max
    }
}

impl Default for RLimit64 {
    fn default() -> Self {
        Self {
            cur: RLIM_INFINITY,
            max: RLIM_INFINITY,
        }
    }
}
