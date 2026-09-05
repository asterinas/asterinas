// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Published mount policy state.
//!
//! A mount policy is the fixed per-mount decision state: whether the mount is
//! effectively read-only or `default_permissions`, the `xino`/UUID modes, the
//! selected private-xattr prefix, and the effective overlay UUID. The
//! upper-filesystem capabilities are measured during mount construction and
//! published here as fixed state; the policy performs no probing itself.

use super::mount::{capabilities::UpperFilesystemCapabilities, inuse::Uuid};
use crate::fs::fs_impls::overlayfs::inode::OverlayXattrPrefix;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UuidMode {
    Off,
    /// Upstream's `uuid=null` mode.
    ///
    /// Locally identical to `Off`: neither handles origin file handles.
    /// Upstream still distinguishes the two — `ovl_origin_uuid` and
    /// `ovl_uuid_match` keep working on `Null` records, so `Null` keeps
    /// writing UUID-bearing origin records while `Off` is the uuid-blind
    /// legacy/clone track — and `Null` is upstream's universal UUID degrade
    /// target.
    Null,
    On,
    /// Reuse an existing persisted UUID, else upgrade to `On`; persistence
    /// failure degrades to `Null`.
    Auto,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum XinoMode {
    Off,
    Auto,
    On,
}

pub(in overlayfs) struct MountPolicy {
    is_effective_read_only: bool,
    uuid: Option<Uuid>,
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    is_default_permissions: bool,
    xattr_prefix: OverlayXattrPrefix,
}

// TODO: A scoped mounter-credential switch is deferred until the VFS provides a credentials API.

impl MountPolicy {
    pub(super) fn assemble(
        is_effective_read_only: bool,
        is_default_permissions: bool,
        xattr_prefix: OverlayXattrPrefix,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            is_default_permissions,
            // Stored even for read-only mounts: the passthrough get/list
            // paths need the selected prefix without an upper.
            xattr_prefix,
        }
    }

    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    pub(in overlayfs) fn is_default_permissions(&self) -> bool {
        self.is_default_permissions
    }

    pub(in overlayfs) fn xattr_prefix(&self) -> OverlayXattrPrefix {
        self.xattr_prefix
    }

    pub(super) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    pub(in overlayfs) fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }
}
