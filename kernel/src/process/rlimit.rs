// SPDX-License-Identifier: MPL-2.0

#![allow(non_camel_case_types)]

use super::process_vm::{INIT_STACK_SIZE, USER_HEAP_SIZE_LIMIT};
use crate::prelude::*;

// Constants for the default rlimit values
// See https://www.man7.org/linux/man-pages/man5/limits.conf.5.html
// FIXME: These values should be used by real users of the kernel.
const MAX_U64: u64 = u64::MAX;
const DEFAULT_RLIMIT_NPROC: u64 = 50;
const DEFAULT_RLIMIT_NOFILE: u64 = 1024;
const DEFAULT_RLIMIT_MEMLOCK: u64 = 64 * 1024 * 1024;
const DEFAULT_RLIMIT_MSGQUEUE: u64 = 819200;
const DEFAULT_RLIMIT_NICE: u64 = 0;
const DEFAULT_RLIMIT_RTPRIO: u64 = 0;

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
        rlimits[ResourceType::RLIMIT_CPU as usize] = RLimit64::new(MAX_U64, MAX_U64);
        rlimits[ResourceType::RLIMIT_FSIZE as usize] = RLimit64::new(MAX_U64, MAX_U64);
        rlimits[ResourceType::RLIMIT_DATA as usize] =
            RLimit64::new(USER_HEAP_SIZE_LIMIT as u64, MAX_U64);
        rlimits[ResourceType::RLIMIT_STACK as usize] =
            RLimit64::new(INIT_STACK_SIZE as u64, MAX_U64);
        rlimits[ResourceType::RLIMIT_CORE as usize] = RLimit64::new(0, MAX_U64);
        rlimits[ResourceType::RLIMIT_RSS as usize] = RLimit64::new(MAX_U64, MAX_U64);
        rlimits[ResourceType::RLIMIT_NPROC as usize] =
            RLimit64::new(DEFAULT_RLIMIT_NPROC, DEFAULT_RLIMIT_NPROC);
        rlimits[ResourceType::RLIMIT_NOFILE as usize] =
            RLimit64::new(DEFAULT_RLIMIT_NOFILE, DEFAULT_RLIMIT_NOFILE);
        rlimits[ResourceType::RLIMIT_MEMLOCK as usize] =
            RLimit64::new(DEFAULT_RLIMIT_MEMLOCK, DEFAULT_RLIMIT_MEMLOCK);
        rlimits[ResourceType::RLIMIT_AS as usize] = RLimit64::new(MAX_U64, MAX_U64);
        rlimits[ResourceType::RLIMIT_LOCKS as usize] = RLimit64::new(MAX_U64, MAX_U64);
        rlimits[ResourceType::RLIMIT_SIGPENDING as usize] =
            RLimit64::new(DEFAULT_RLIMIT_NPROC, DEFAULT_RLIMIT_NPROC);
        rlimits[ResourceType::RLIMIT_MSGQUEUE as usize] =
            RLimit64::new(DEFAULT_RLIMIT_MSGQUEUE, DEFAULT_RLIMIT_MSGQUEUE);
        rlimits[ResourceType::RLIMIT_NICE as usize] =
            RLimit64::new(DEFAULT_RLIMIT_NICE, DEFAULT_RLIMIT_NICE);
        rlimits[ResourceType::RLIMIT_RTPRIO as usize] =
            RLimit64::new(DEFAULT_RLIMIT_RTPRIO, DEFAULT_RLIMIT_RTPRIO);
        rlimits[ResourceType::RLIMIT_RTTIME as usize] = RLimit64::new(MAX_U64, MAX_U64);

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
            cur: MAX_U64,
            max: MAX_U64,
        }
    }
}
