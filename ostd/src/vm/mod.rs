// SPDX-License-Identifier: MPL-2.0

//! Guest execution and physical memory management.

pub mod gpm_space;
mod interrupt;
mod timer;

pub use gpm_space::GuestPhysMemSpace;
pub use interrupt::GuestInterruptPort;
pub use timer::GuestTimerPort;

pub use crate::arch::vm::{GuestMode, GuestRunResult};

/// A guest physical address.
pub type Gpaddr = usize;
