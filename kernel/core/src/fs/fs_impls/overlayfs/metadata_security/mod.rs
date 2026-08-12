// SPDX-License-Identifier: MPL-2.0

//! The module root of the metadata, permission, and xattr policy subsystem.
//!
//! This module declares the three `metadata_security/*` submodules and hosts
//! the thin cross-file helpers: the single private delegation helper
//! `OverlayInode::delegate_to_real` (shared by the three sibling files) and
//! the cross-module `OverlayFs::xattr_policy` accessor (the field lives in
//! `mount/superblock.rs` and stores the policy type defined in the sibling
//! `xattr.rs`). The real control flow lives in the sibling files:
//! `permission.rs` (two-stage permission pipeline), `metadata.rs` (metadata
//! setters), and `xattr.rs` (`OverlayXattrPolicy`/`XattrClass` + the xattr
//! entries).
//!
//! Lock contract: this module acquires no Overlay lock domain.
//! `delegate_to_real` re-resolves the current authority per call through
//! `select_real_inode()` (a brief `INODE` facts snapshot, released before the
//! underlying call) and runs the delegation under the mount's
//! creator-credential scope (`with_creator_credentials_fn`); no Overlay lock
//! is held across any underlying permission/MAC/xattr callback.

use self::xattr::OverlayXattrPolicy;
use super::{mount::OverlayFs, projection::OverlayInode};
use crate::{fs::vfs::inode::Inode, prelude::*};

mod metadata;
mod permission;
pub(super) mod xattr;

impl OverlayInode {
    /// The single private delegation helper of this module tree.
    ///
    /// Resolves the current real authority once — a fresh per-call
    /// `select_real_inode()`, so an fd opened while lower-backed observes the
    /// upper real inode on its next operation after a copy-up — and runs
    /// `operation_fn` under the mount's creator-credential scope
    /// (`with_creator_credentials_fn`). The returned `Arc<dyn Inode>` strong
    /// pin keeps the resolved real inode alive for the delegation; no Overlay
    /// lock is held across the underlying call. The permission stage has
    /// already admitted the operation (or the entry is a pure read
    /// delegation), so the forward runs directly under the
    /// creator-credential scope; for metadata setters whose underlying real
    /// ops do not self-evaluate, `check_real_permission` ran the explicit
    /// real check before this forward.
    fn delegate_to_real<T>(
        &self,
        operation_fn: impl FnOnce(&Arc<dyn Inode>) -> Result<T>,
    ) -> Result<T> {
        let fs = self.fs_arc()?;
        let real = self.select_real_inode();
        fs.policy()
            .credential_policy()
            .with_creator_credentials_fn(|| operation_fn(&real))
    }
}

impl OverlayFs {
    /// Returns the immutable xattr classification policy: the stateless
    /// [`OverlayXattrPolicy`] (owned once, no lock) consumed by this module's
    /// xattr entries (the `list_xattr` private-name filter) and by the
    /// copy-up copy-time xattr filter.
    pub(super) fn xattr_policy(&self) -> &OverlayXattrPolicy {
        &self.xattr_policy
    }
}
