// SPDX-License-Identifier: MPL-2.0

//! Published mount policy state.
//!
//! The upper-filesystem capabilities are measured during mount construction
//! and published here as fixed state; the policy performs no probing itself.

#![short_vis_path::add(overlayfs)]

use super::{
    capabilities::{UpperFilesystemCapabilities, WhiteoutCapability},
    inuse::Uuid,
};
use crate::fs::vfs::xattr::XattrNamespace;

/// The per-mount decisions a mount settles once during construction and publishes as fixed state.
pub(in overlayfs) struct MountPolicy {
    /// Mutating permission checks fail with `EROFS` while set; covers read-only mounts and uppers.
    is_effective_read_only: bool,
    /// The effective overlay UUID published as the superblock fsid; `None` when none persists.
    uuid: Option<Uuid>,
    /// Upper capabilities probed after the claim; `None` for lower-only and read-only mounts.
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    /// The xattr namespace for the overlay's own records; kept even for read-only mounts.
    xattr_namespace: XattrNamespace,
}

impl MountPolicy {
    pub(super) fn assemble(
        is_effective_read_only: bool,
        xattr_namespace: XattrNamespace,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            xattr_namespace,
        }
    }

    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    pub(in overlayfs) fn xattr_namespace(&self) -> XattrNamespace {
        self.xattr_namespace
    }

    pub(in super::super) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }

    /// Returns the whiteout form the upper can publish; `Unsupported` means no form.
    pub(in overlayfs) fn whiteout_capability(&self) -> WhiteoutCapability {
        self.upper_capabilities()
            .map_or(WhiteoutCapability::Unsupported, |capabilities| {
                capabilities.whiteout_capability()
            })
    }

    /// Returns whether the probed upper can store the overlay's private xattrs.
    pub(in overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.upper_capabilities
            .is_some_and(|caps| caps.can_store_private_xattr())
    }

    pub(in overlayfs) fn can_express_whiteout(&self) -> bool {
        self.whiteout_capability() != WhiteoutCapability::Unsupported
    }
}
