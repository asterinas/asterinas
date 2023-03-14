//! This implementation is from occlum

#![allow(non_camel_case_types)]

use crate::prelude::*;

use super::{process_vm::user_heap::USER_HEAP_SIZE_LIMIT, program_loader::elf::INIT_STACK_SIZE};

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
#[derive(Debug, Clone, Copy)]
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

impl TryFrom<u32> for ResourceType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(ResourceType::RLIMIT_CPU),
            1 => Ok(ResourceType::RLIMIT_FSIZE),
            2 => Ok(ResourceType::RLIMIT_DATA),
            3 => Ok(ResourceType::RLIMIT_STACK),
            4 => Ok(ResourceType::RLIMIT_CORE),
            5 => Ok(ResourceType::RLIMIT_RSS),
            6 => Ok(ResourceType::RLIMIT_NPROC),
            7 => Ok(ResourceType::RLIMIT_NOFILE),
            8 => Ok(ResourceType::RLIMIT_MEMLOCK),
            9 => Ok(ResourceType::RLIMIT_AS),
            10 => Ok(ResourceType::RLIMIT_LOCKS),
            11 => Ok(ResourceType::RLIMIT_SIGPENDING),
            12 => Ok(ResourceType::RLIMIT_MSGQUEUE),
            13 => Ok(ResourceType::RLIMIT_NICE),
            14 => Ok(ResourceType::RLIMIT_RTPRIO),
            15 => Ok(ResourceType::RLIMIT_RTTIME),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid resource type"),
        }
    }
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
        Self {
            cur,
            max: u64::max_value(),
        }
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
            cur: u64::max_value(),
            max: u64::max_value(),
        }
    }
}
