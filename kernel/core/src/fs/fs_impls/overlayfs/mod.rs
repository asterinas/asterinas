// SPDX-License-Identifier: MPL-2.0

mod copyup;
mod dir;
mod metadata_security;
mod mount;
mod projection;
mod readdir_index;

/// The mutating-vs-read-only access class of an overlayfs entry.
///
/// Closed set: encodes the coarse mutating-vs-read-only class that entries
/// derive from the VFS surface, replacing a boolean parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::fs::fs_impls::overlayfs) enum AccessType {
    /// open/access/exec/metadata-read/xattr-read: no EROFS gate, no promotion.
    ReadOnly,
    /// chmod/chown/utimes, xattr set/remove: EROFS gate + `ensure_upper_authority()`.
    Mutating,
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&mount::OverlayFsType).unwrap();
}
