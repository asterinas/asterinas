// SPDX-License-Identifier: MPL-2.0

//! File operation attributes and flags.
//!
//! This module defines the parameters that control the behavior of file
//! operations, such as `open` and `fcntl`. These flags specify the desired
//! access mode (e.g., read-only), I/O behavior (e.g., non-blocking),
//! and creation semantics, forming the contract between user-space requests
//! and VFS actions.

mod access_mode;
mod creation_flags;
mod open_args;
mod status_flags;

pub use access_mode::AccessMode;
pub use creation_flags::CreationFlags;
pub use open_args::OpenArgs;
pub use status_flags::{AtomicStatusFlags, StatusFlags};
