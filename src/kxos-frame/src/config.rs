#![allow(unused)]

use crate::log::LogLevel;

pub const USER_STACK_SIZE: usize = PAGE_SIZE * 2;
pub const KERNEL_STACK_SIZE: usize = PAGE_SIZE * 16;
pub const KERNEL_HEAP_SIZE: usize = 0x1_000_000;

pub const KERNEL_OFFSET: usize = 0xffffff00_00000000;
pub const PHYS_OFFSET: usize = 0xFFFF800000000000;
pub const ENTRY_COUNT: usize = 512;

pub const PAGE_SIZE: usize = 0x1000;
pub const PAGE_SIZE_BITS: usize = 0xc;

pub const KVA_START: usize = (usize::MAX) << PAGE_SIZE_BITS;

pub const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;
