// SPDX-License-Identifier: MPL-2.0

//! Provides FUSE protocol definitions shared by in-kernel clients.
//!
//! This crate contains the transport-independent on-wire pieces of the FUSE
//! protocol: request and reply headers, payload layouts, opcodes, flags, and
//! common constants.
//!
//! The main entry points are:
//!
//! - [`FuseOperation`], which describes one typed FUSE request/reply pair.
//! - [`FuseError`] and [`FuseResult`], which report encoding and decoding
//!   failures.
//! - POD-compatible protocol structs such as [`ReqHeader`] and [`ReplyHeader`].
//! - Per-operation request and reply structs under [`mod@ops`].
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

mod attr;
mod dirent;
mod error;
mod header;
mod ids;
mod operation;
pub mod ops;
mod status;

pub use self::{
    attr::{Attr, EntryReply},
    dirent::{DirOffset, Dirent, DirentType, FuseDirEntry},
    error::{FuseError, FuseResult},
    header::{ReplyHeader, ReqHeader},
    ids::{FuseFileHandle, FuseGeneration, FuseNodeId, FuseUnique, LookupCount},
    operation::{FuseOpcode, FuseOperation, ReplyExpectation},
    ops::{
        create::{CreateOperation, CreateReq},
        forget::{ForgetOperation, ForgetReq},
        getattr::{FuseAttrReply, GetattrFlags, GetattrOperation, GetattrReq},
        init::{FuseInitFlags, FuseInitFlags2, InitOperation, InitReply, InitReq},
        link::{LinkOperation, LinkReq},
        lookup::LookupOperation,
        lseek::{LseekOperation, LseekReply, LseekReq},
        mkdir::{MkdirOperation, MkdirReq},
        mknod::{MknodOperation, MknodReq},
        open::{FuseOpenFlags, OpenOperation, OpenReply, OpenReq, OpendirOperation},
        read::{ReadOperation, ReadReq},
        readdir::ReaddirOperation,
        readlink::{MAX_READLINK_LEN, ReadlinkOperation},
        release::{ReleaseFlags, ReleaseKind, ReleaseOperation, ReleaseReq},
        rename::{RenameOperation, RenameReq},
        rmdir::RmdirOperation,
        setattr::{SetattrOperation, SetattrReq, SetattrValid},
        statfs::{Kstatfs, StatfsOperation, StatfsReply},
        unlink::UnlinkOperation,
        write::{WriteFlags, WriteOperation, WriteReply, WriteReq},
    },
    status::{FuseCompleteFn, FuseCompletion, FuseStatus},
};

/// The root inode ID used by the FUSE protocol.
pub const FUSE_ROOT_ID: FuseNodeId = FuseNodeId::new(1);

/// The major FUSE protocol version supported by this crate.
pub const FUSE_KERNEL_VERSION: u32 = 7;

/// The minor FUSE protocol version supported by this crate.
pub const FUSE_KERNEL_MINOR_VERSION: u32 = 38;

/// Minimum `max_write` value enforced by the client.
///
/// Even if the server reports a smaller `max_write` in `FUSE_INIT`, the client
/// uses at least one page (4096 bytes) per write request.
pub const MIN_MAX_WRITE: u32 = 4096;
