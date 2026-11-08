// SPDX-License-Identifier: MPL-2.0

//! Miscellaneous filesystem utilities shared across `fs` modules.

pub(crate) use dirent_visitor::DirentVisitor;
pub(crate) use direntry_vec::DirEntryVecExt;
pub(crate) use endpoint::{Endpoint, EndpointState};
pub(crate) use id_bitmap::IdBitmap;

use crate::{
    process::{Gid, Uid, posix_thread::AsPosixThread},
    thread::Thread,
};

mod dirent_visitor;
mod direntry_vec;
mod endpoint;
mod id_bitmap;
pub(crate) mod systree_inode;

/// Maximum bytes in a path
pub(crate) const PATH_MAX: usize = 4096;

/// Maximum bytes in a file name
pub(crate) const NAME_MAX: usize = 255;

/// The upper limit for resolving symbolic links
pub(crate) const SYMLINKS_MAX: usize = 40;

pub(crate) type CStr256 = aster_util::fixed_str::FixedCStr<256>;
pub(crate) type Str16 = aster_util::fixed_str::FixedNonTerminatedCStr<16>;
pub(crate) type Str64 = aster_util::fixed_str::FixedNonTerminatedCStr<64>;

/// Returns the current thread's filesystem IDs, or root IDs for kernel threads.
pub(in crate::fs) fn current_fs_ids() -> (Uid, Gid) {
    Thread::current()
        .and_then(|thread| {
            let posix_thread = thread.as_posix_thread()?;
            let credentials = posix_thread.credentials();
            Some((credentials.fsuid(), credentials.fsgid()))
        })
        .unwrap_or((Uid::new_root(), Gid::new_root()))
}
