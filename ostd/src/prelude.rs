// SPDX-License-Identifier: MPL-2.0

//! The prelude.

/// A specialized [`Result`] type for this crate.
///
/// [`Result`]: core::result::Result
#[expect(
    unused_qualifications,
    reason = "`error` below is intended to re-export the macro but brings the module into scope"
)]
pub type Result<T, E = crate::error::Error> = core::result::Result<T, E>;

pub(crate) use alloc::{boxed::Box, sync::Arc, vec::Vec};

#[cfg(ktest)]
pub use ostd_macros::ktest;

pub use crate::{
    alert, crit, debug, early_print as print, early_println as println, emerg, error, info,
    mm::{HasPaddr, HasSize, Paddr, Vaddr},
    notice,
    panic::abort,
    warn,
};
