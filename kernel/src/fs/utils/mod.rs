// SPDX-License-Identifier: MPL-2.0

//! Miscellaneous filesystem utilities shared across `fs` modules.

pub use dirent_visitor::{DirentCounter, DirentVisitor};
pub use direntry_vec::DirEntryVecExt;
pub use endpoint::{Endpoint, EndpointState};
pub use id_bitmap::IdBitmap;

mod dirent_visitor;
mod direntry_vec;
mod endpoint;
mod id_bitmap;
pub mod systree_inode;

/// Maximum bytes in a path
pub const PATH_MAX: usize = 4096;

/// Maximum bytes in a file name
pub const NAME_MAX: usize = 255;

/// The upper limit for resolving symbolic links
pub const SYMLINKS_MAX: usize = 40;

pub type CStr256 = aster_util::fixed_str::FixedCStr<256>;
pub type Str16 = aster_util::fixed_str::FixedNonTerminatedCStr<16>;
pub type Str64 = aster_util::fixed_str::FixedNonTerminatedCStr<64>;
