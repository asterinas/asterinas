// SPDX-License-Identifier: MPL-2.0

//! The prelude.

pub type Result<T> = core::result::Result<T, crate::error::Error>;

pub(crate) use alloc::{boxed::Box, sync::Arc, vec::Vec};
pub(crate) use core::any::Any;

pub use crate::vm::{Paddr, Vaddr};
