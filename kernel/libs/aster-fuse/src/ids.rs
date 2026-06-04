// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

/// A FUSE request identifier (`unique`) exchanged on the protocol.
///
/// The client assigns one `FuseUnique` to each request and the server copies it
/// into the matching reply. The value `0` is reserved for unsolicited
/// notifications rather than ordinary request/reply matching.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod)]
pub struct FuseUnique(u64);

impl FuseUnique {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// An opaque FUSE file handle issued by the server.
///
/// The server returns this handle in `FUSE_OPEN` and `FUSE_OPENDIR` replies.
/// Subsequent I/O and release requests carry it so the server can locate the
/// corresponding open-file state.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod)]
pub struct FuseFileHandle(u64);

impl FuseFileHandle {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

/// A FUSE inode identifier (`nodeid`) exchanged on the protocol.
///
/// A `FuseNodeId` identifies one server-side inode object. It appears in FUSE
/// request headers and in lookup-like replies such as [`crate::EntryReply`].
///
/// For a cached VFS inode, this value is an immutable identity binding: if
/// lookup revalidation later resolves the same pathname to a different
/// `FuseNodeId`, the old cached inode must be treated as stale and replaced
/// instead of being retargeted in place.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Pod)]
pub struct FuseNodeId(u64);

impl FuseNodeId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// A FUSE inode generation number exchanged on the protocol.
///
/// FUSE lookup-like replies pair this value with a [`FuseNodeId`] so clients can
/// distinguish a reused inode number from the old cached inode.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub struct FuseGeneration(u64);

impl FuseGeneration {
    /// Creates a generation number from its raw protocol value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw protocol value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Client-side mirror of the server's nlookup for an inode.
///
/// In the FUSE protocol, every reply that contains an [`crate::EntryReply`]
/// increments the server-side nlookup by one. This includes `FUSE_LOOKUP`,
/// `FUSE_CREATE`, `FUSE_MKDIR`, `FUSE_MKNOD`, and `FUSE_LINK`. This type
/// tracks the same count on the client side so that the accumulated value can
/// be sent back via `FUSE_FORGET` when the inode is dropped.
#[repr(transparent)]
pub struct LookupCount(AtomicU64);

impl LookupCount {
    /// Creates a counter seeded with the lookup reference carried by an
    /// [`crate::EntryReply`] reply.
    pub const fn initial() -> Self {
        Self(AtomicU64::new(1))
    }

    /// Acquires one lookup reference after receiving an `EntryReply` reply.
    pub fn acquire(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    /// Returns the value to pass to `FUSE_FORGET` and clears the counter.
    pub fn drain(&self) -> u64 {
        self.0.swap(0, Ordering::AcqRel)
    }
}
