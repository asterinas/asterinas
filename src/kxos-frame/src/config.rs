#![allow(unused)]

pub const USER_STACK_SIZE: usize = PAGE_SIZE * 2;
pub const KERNEL_STACK_SIZE: usize = PAGE_SIZE * 16;
pub const KERNEL_HEAP_SIZE: usize = 0x1_000_000;

pub const KERNEL_OFFSET: usize = 0xffffff00_00000000;
pub const PHYS_OFFSET: usize = 0xFFFF800000000000;
pub const ENTRY_COUNT: usize = 512;

pub const PAGE_SIZE: usize = 0x1000;
pub const PAGE_SIZE_BITS: usize = 0xc;
pub const MEM_START: usize = 0x8000_0000;

pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;
pub const TRAP_CONTEXT_BASE: usize = TRAMPOLINE - PAGE_SIZE;

pub const KVA_START: usize = (usize::MAX) << PAGE_SIZE_BITS;
