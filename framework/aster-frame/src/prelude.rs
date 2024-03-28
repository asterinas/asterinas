// SPDX-License-Identifier: MPL-2.0

//! The prelude.

pub type Result<T> = core::result::Result<T, crate::error::Error>;

pub(crate) use alloc::{boxed::Box, sync::Arc, vec::Vec};
pub(crate) use core::any::Any;

pub use aster_main::aster_main;

pub use crate::{
    early_print as print, early_println as println,
    panicking::abort,
    task::atomic::{
        atomic_procedure, enter_atomic_mode, might_break, might_break_atomic_mode, AtomicModeGuard,
    },
    vm::{Paddr, Vaddr},
};
