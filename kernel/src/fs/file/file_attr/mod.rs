// SPDX-License-Identifier: MPL-2.0

//! File operation attributes and flags.
//!
//! This module defines the parameters that control the behavior of file
//! operations, such as `open` and `fcntl`. These flags specify the desired
//! access mode (e.g., read-only), I/O behavior (e.g., non-blocking),
//! and creation semantics, forming the contract between user-space requests
//! and VFS actions.

pub(super) mod access_mode;
pub(super) mod creation_flags;
pub(super) mod open_args;
pub(super) mod status_flags;
