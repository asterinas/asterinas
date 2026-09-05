// SPDX-License-Identifier: MPL-2.0

//! Published mount policy state.
//!
//! A mount policy is the fixed per-mount decision state: whether the mount is
//! effectively read-only or `default_permissions`, the `xino`/UUID modes, the
//! selected private-xattr prefix, and the effective overlay UUID. The
//! upper-filesystem capabilities are measured during mount construction and
//! published here as fixed state; the policy performs no probing itself.

#![short_vis_path::add(overlayfs)]

use super::mount::{
    capabilities::{UpperFilesystemCapabilities, WhiteoutCapability},
    inuse::Uuid,
};
use crate::fs::vfs::xattr::XattrNamespace;

/// Mount-fixed decisions: permission modes, xattr namespace, UUID, and upper capabilities.
pub(in overlayfs) struct MountPolicy {
    /// Mutating permission checks fail with `EROFS` while set; covers read-only mounts and uppers.
    is_effective_read_only: bool,
    /// The effective overlay UUID published as the superblock fsid; `None` when none persists.
    uuid: Option<Uuid>,
    /// Upper capabilities probed after the claim; `None` for lower-only and read-only mounts.
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    /// When set, the check stops after the local gate and skips the real-authority check.
    is_default_permissions: bool,
    /// The xattr namespace for the overlay's own records; kept even for read-only mounts.
    xattr_namespace: XattrNamespace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UuidMode {
    /// UUID-blind mode: no persistent overlay UUID is written or matched.
    Off,
    /// `uuid=null` is accepted for compatibility and behaves like `Off` (no UUID record).
    Null,
    /// Requires a persistent UUID: reuse the upper's, else create and persist one.
    On,
    /// Reuses an existing UUID, else upgrades to `On`; persistence failure degrades to `Null`.
    Auto,
}

/// The `xino` option: whether the overlay publishes its own device id with a layer-encoded ino.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum XinoMode {
    /// Never encode; a non-directory publishes the underlying dev/ino.
    Off,
    /// Encode only when the layers span several underlying filesystems.
    Auto,
    /// Forces the encoding attempt; objects that do not fit fall back to xino-off.
    On,
}

// TODO: A scoped mounter-credential switch is deferred until the VFS provides a credentials API.

impl MountPolicy {
    pub(super) fn assemble(
        is_effective_read_only: bool,
        is_default_permissions: bool,
        xattr_namespace: XattrNamespace,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            is_default_permissions,
            xattr_namespace,
        }
    }

    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    pub(in overlayfs) fn is_default_permissions(&self) -> bool {
        self.is_default_permissions
    }

    pub(in overlayfs) fn xattr_namespace(&self) -> XattrNamespace {
        self.xattr_namespace
    }

    pub(super) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    pub(in overlayfs) fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
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
}
