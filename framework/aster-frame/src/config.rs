#![allow(unused)]

use log::Level;

pub const USER_STACK_SIZE: usize = PAGE_SIZE * 4;
pub const KERNEL_STACK_SIZE: usize = PAGE_SIZE * 64;
pub const KERNEL_HEAP_SIZE: usize = PAGE_SIZE * 256;

pub const KERNEL_OFFSET: usize = 0xffffffff80000000;

pub const PHYS_OFFSET: usize = 0xFFFF800000000000;
pub const ENTRY_COUNT: usize = 512;

pub const PAGE_SIZE: usize = 0x1000;
pub const PAGE_SIZE_BITS: usize = 0xc;

pub const KVA_START: usize = (usize::MAX) << PAGE_SIZE_BITS;

pub const DEFAULT_LOG_LEVEL: Level = Level::Error;
/// Whether to enable the debug log in scheduling.
/// Only work when `DEFAULT_LOG_LEVEL` is `Debug`.
/// Excessive scheduling logs can overwhelm other logs, so disabled by default.
pub const SCHED_DEBUG_LOG: bool = false;
/// This value represent the base timer frequency in Hz
pub const TIMER_FREQ: u64 = 500;

pub const REAL_TIME_TASK_PRI: u16 = 100;

/// Whether the child process should run first for the first time in scheduling.
/// This option provides a preference for scheduling, not a guarantee.
/// todo: defaulted as false
pub const CHILD_RUN_FIRST: bool = true;
