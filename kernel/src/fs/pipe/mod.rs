// SPDX-License-Identifier: MPL-2.0

//! Pipes implementation.
//!
//! This module provides both anonymous and named pipes for inter-process communication.

pub use anon_pipe::new_file_pair;
pub(in crate::fs) use common::{Pipe, PipeHandle, check_status_flags};

mod anon_pipe;
mod common;
