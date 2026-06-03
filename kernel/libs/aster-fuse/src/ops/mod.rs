// SPDX-License-Identifier: MPL-2.0

//! Per-operation FUSE request/reply definitions and encoders.
//!
//! Each submodule corresponds to one FUSE operation and owns both the on-wire
//! structures and the `FuseOperation` implementation used to serialize a
//! request and parse its reply. Types shared across operations (headers,
//! `Attr`, `EntryReply`, directory entries) live in the crate root.
//!
//! Operation payload types use the `Req` suffix for request payloads sent to
//! the server and the `Reply` suffix for reply payloads returned by the server.
//!
//! Operation wrapper types use the `Operation` suffix and implement [`crate::FuseOperation`].

mod util;

pub mod create;
pub mod forget;
pub mod getattr;
pub mod init;
pub mod link;
pub mod lookup;
pub mod lseek;
pub mod mkdir;
pub mod mknod;
pub mod open;
pub mod read;
pub mod readdir;
pub mod readlink;
pub mod release;
pub mod rename;
pub mod rmdir;
pub mod setattr;
pub mod statfs;
pub mod unlink;
pub mod write;
