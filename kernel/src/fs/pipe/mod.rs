// SPDX-License-Identifier: MPL-2.0

//! Pipes implementation.
//!
//! This module provides both anonymous and named pipes for inter-process communication.

pub use anony_pipe::{new_file_pair, AnonPipeFile, AnonPipeInode};
pub use named_pipe::NamedPipe;

mod anony_pipe;
mod common;
mod named_pipe;
