//! The prelude.

pub type Result<T> = core::result::Result<T, crate::error::Error>;

pub(crate) use alloc::boxed::Box;
pub(crate) use alloc::sync::Arc;
pub(crate) use alloc::vec::Vec;
pub(crate) use core::any::Any;

pub use crate::vm::{Paddr, Vaddr};
