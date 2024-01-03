//! The constants

use crate::prelude::*;

/// The max number of arguments that can be used to creating a new process.
pub const MAX_ARGV_NUMBER: usize = 128;
/// The max number of environmental variables that can be used to creating a new process.
pub const MAX_ENVP_NUMBER: usize = 128;
/// The max length of each argument to create a new process.
pub const MAX_ARG_LEN: usize = 2048;
/// The max length of each environmental variable (the total length of key-value pair) to create a new process.
pub const MAX_ENV_LEN: usize = 128;

/// The base address of user heap
pub(super) const USER_HEAP_BASE: Vaddr = 0x0000_0000_1000_0000;
/// The max allowed size of user heap
pub(super) const USER_HEAP_SIZE_LIMIT: usize = PAGE_SIZE * 1000;
