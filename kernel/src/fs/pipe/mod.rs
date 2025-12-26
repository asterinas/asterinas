// SPDX-License-Identifier: MPL-2.0

//! Pipes implementation.
//!
//! This module provides both anonymous and named pipes for inter-process communication.

pub(super) use anon_pipe::AnonPipeInode;
pub use anon_pipe::new_file_pair;
pub(super) use common::{Pipe, PipeHandle, check_status_flags};

mod anon_pipe;
mod common;
