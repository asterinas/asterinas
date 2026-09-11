// SPDX-License-Identifier: MPL-2.0

//! Guest physical memory management.

pub mod gpm_space;

pub use gpm_space::GuestPhysMemSpace;

/// A guest physical address.
pub type Gpaddr = usize;
