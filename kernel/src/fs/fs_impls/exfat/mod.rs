// SPDX-License-Identifier: MPL-2.0

//! exFAT filesystem runtime pieces.
//!
//! This module is the filesystem boundary for Asterinas' exFAT implementation.
//! It gathers the filesystem owner, on-disk decoders, allocation structures,
//! inode runtime, and up-case support
//! without exposing a stable public exFAT API outside the kernel filesystem tree.
//!
//! The internal module map is:
//! `fs` for filesystem lifetime and VFS registration;
//! `boot`, `fat`, `bitmap`, and `dir_entry_format` for on-disk structures and validation;
//! `inode` for the inode runtime and mutation paths;
//! and `upcase` for case-folding support.
//!
//! This implementation currently supports the exFAT mount/runtime surface
//! used by Asterinas.
//! Unsupported features and malformed on-disk layouts are rejected through local error
//! constructors rather than compatibility shims.
//!
//! Authoritative references are Microsoft's
//! [exFAT File System Specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification),
//! Sections 2 through 8,
//! plus the owner module boundaries in `crate::fs::fs_impls::exfat::fs`
//! and `crate::fs::fs_impls::exfat::inode`.

use crate::prelude::*;

mod bitmap;
mod boot;
mod dir_entry_format;
mod fat;
mod fs;
mod inode;
mod upcase;

pub(super) use fs::init;

fn device_io() -> Error {
    Error::new(Errno::EIO)
}

fn inconsistent_bitmap_accounting() -> Error {
    Error::with_message(Errno::EUCLEAN, "exFAT bitmap accounting mismatch")
}

fn invalid_on_disk_layout() -> Error {
    Error::with_message(Errno::EUCLEAN, "corrupt exFAT on-disk layout")
}

fn invalid_operation_input() -> Error {
    Error::new(Errno::EINVAL)
}

fn not_mounted() -> Error {
    Error::with_message(Errno::EINVAL, "filesystem is not mounted")
}
