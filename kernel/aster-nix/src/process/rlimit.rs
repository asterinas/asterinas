// SPDX-License-Identifier: MPL-2.0

#![allow(non_camel_case_types)]

use super::process_vm::{INIT_STACK_SIZE, USER_HEAP_SIZE_LIMIT};
use crate::prelude::*;

pub struct ResourceLimits {
    rlimits: [RLimit64; RLIMIT_COUNT],
}

impl ResourceLimits {
    pub fn get_rlimit(&self, resource: ResourceType) -> &RLimit64 {
        &self.rlimits[resource as usize]
    }

    pub fn get_rlimit_mut(&mut self, resource: ResourceType) -> &mut RLimit64 {
        &mut self.rlimits[resource as usize]
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        let stack_size = RLimit64::new(INIT_STACK_SIZE as u64);
        let heap_size = RLimit64::new(USER_HEAP_SIZE_LIMIT as u64);
        let open_files = RLimit64::new(1024);

        let mut rlimits = Self {
            rlimits: [RLimit64::default(); RLIMIT_COUNT],
        };
        *rlimits.get_rlimit_mut(ResourceType::RLIMIT_STACK) = stack_size;
        *rlimits.get_rlimit_mut(ResourceType::RLIMIT_DATA) = heap_size;
        *rlimits.get_rlimit_mut(ResourceType::RLIMIT_NOFILE) = open_files;
        rlimits
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
    pub fn new(cur: u64) -> Self {
        Self { cur, max: u64::MAX }
    }

    pub fn get_cur(&self) -> u64 {
        self.cur
    }

    pub fn get_max(&self) -> u64 {
        self.max
    }
}

impl Default for RLimit64 {
    fn default() -> Self {
        Self {
            cur: u64::MAX,
            max: u64::MAX,
        }
    }
}
