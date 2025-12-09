// SPDX-License-Identifier: MPL-2.0

//! Pipes implementation.
//!
//! This module provides both anonymous and named pipes for inter-process communication.

pub use anon_pipe::{AnonPipeFile, AnonPipeInode, new_file_pair};
pub use common::Pipe;

mod anon_pipe;
mod common;
