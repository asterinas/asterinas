// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Published mount policy state.
//!
//! A mount policy is the fixed per-mount decision state: whether the mount is
//! effectively read-only or `default_permissions`, the `xino`/UUID modes, and
//! the effective overlay UUID. The upper-filesystem capabilities are the
//! probe-derived limits of the post-claim upper filesystem.
//!
//! This module owns the [`MountPolicy`] assembled by
//! [`OverlayFs`](crate::fs::fs_impls::overlayfs::superblock::OverlayFs) and
//! the [`UpperFilesystemCapabilities`].

use super::{
    claims::{OVERLAY_UUID_SIZE, Uuid},
    options::{MountOptions, UuidMode, XinoMode},
};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::metadata_security::xattr::uuid_xattr_name,
        utils::DirentVisitor,
        vfs::{
            inode::{Inode, MknodType},
            path::is_dot_or_dotdot,
        },
    },
    prelude::*,
};

const CHAR_DEVICE_PROBE_PREFIX: &str = ".overlay-char-device-probe-";
const D_TYPE_PROBE_PREFIX: &str = ".overlay-dtype-probe-";

pub(in overlayfs) struct MountPolicy {
    is_effective_read_only: bool,
    uuid: Option<Uuid>,
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    is_default_permissions: bool,
    xino_mode: XinoMode,
}

// TODO: Reintroduce a scoped creator-credential switch once the VFS provides a credentials API.

impl MountPolicy {
    pub(super) fn assemble(
        is_effective_read_only: bool,
        options: &MountOptions,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            is_default_permissions: options.is_default_permissions,
            xino_mode: options.xino_mode.unwrap_or(XinoMode::Auto),
        }
    }

    /// Returns whether this mount is effectively read-only.
    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    /// Returns whether `default_permissions` was specified for this mount.
    pub(in overlayfs) fn is_default_permissions(&self) -> bool {
        self.is_default_permissions
    }

    pub(super) fn xino_mode(&self) -> XinoMode {
        self.xino_mode
    }

    /// Returns the overlay UUID when effective.
    pub(in overlayfs) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    /// Returns the post-claim upper-filesystem capabilities.
    pub(in overlayfs) fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) struct UpperFilesystemCapabilities {
    can_store_private_xattr: bool,
    can_report_directory_type: bool,
    can_mknod_char: bool,
}

impl UpperFilesystemCapabilities {
    /// Probes the upper/workspace capabilities post-claim (writable mounts
    /// only, sleep-capable construction context).
    pub(super) fn probe(
        upper_inode: &Arc<dyn Inode>,
        workspace_inode: &Arc<dyn Inode>,
    ) -> Result<Self> {
        // The d_type and char-device probes create uniquely-named temp
        // entries in the workdir staging workspace and remove them on
        // success/failure.
        let can_store_private_xattr = Self::probe_private_xattr(upper_inode)?;
        let can_report_directory_type = Self::probe_d_type(workspace_inode)?;
        let can_mknod_char = Self::probe_mknod_char(workspace_inode)?;
        Ok(Self {
            can_store_private_xattr,
            can_report_directory_type,
            can_mknod_char,
        })
    }

    fn probe_private_xattr(upper_inode: &Arc<dyn Inode>) -> Result<bool> {
        let name = uuid_xattr_name()?;
        let mut value = [0u8; OVERLAY_UUID_SIZE];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper_inode.get_xattr(name, &mut writer) {
            Ok(_) => Ok(true),
            Err(err) if err.error() == Errno::ENODATA => Ok(true),
            Err(err) if err.error() == Errno::ERANGE => Ok(true),
            Err(err) if err.error() == Errno::EOPNOTSUPP => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn probe_d_type(workspace_inode: &Arc<dyn Inode>) -> Result<bool> {
        let d_type_probe_name =
            crate::fs::fs_impls::overlayfs::workdir::workdir_temp_name(D_TYPE_PROBE_PREFIX);
        workspace_inode.create(&d_type_probe_name, InodeType::File, InodeMode::empty())?;
        let mut d_type_probe = DTypeProbeVisitor::new();
        let mut offset = 0;
        let d_type_scan_result = loop {
            match workspace_inode.readdir_at(offset, &mut d_type_probe) {
                Ok(0) => break Ok(()),
                Ok(visited) => offset += visited,
                Err(err) => break Err(err),
            }
        };
        match d_type_scan_result {
            Ok(()) => {
                workspace_inode.unlink(&d_type_probe_name)?;
                Ok(!d_type_probe.saw_unknown_non_dot)
            }
            Err(err) => {
                let _ = workspace_inode.unlink(&d_type_probe_name);
                Err(err)
            }
        }
    }

    fn probe_mknod_char(workspace_inode: &Arc<dyn Inode>) -> Result<bool> {
        let probe_name =
            crate::fs::fs_impls::overlayfs::workdir::workdir_temp_name(CHAR_DEVICE_PROBE_PREFIX);
        match workspace_inode.mknod(&probe_name, InodeMode::empty(), MknodType::CharDevice(0)) {
            Ok(_) => {
                workspace_inode.unlink(&probe_name)?;
                Ok(true)
            }
            Err(err)
                if matches!(
                    err.error(),
                    Errno::EOPNOTSUPP | Errno::EPERM | Errno::EACCES
                ) =>
            {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Consumed by the origin-record store.
    pub(in overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.can_store_private_xattr
    }

    pub(super) fn can_report_directory_type(&self) -> bool {
        self.can_report_directory_type
    }

    /// Reports whether the workdir supports the classic whiteout char device
    /// `0:0`.
    pub(in overlayfs) fn can_mknod_char(&self) -> bool {
        self.can_mknod_char
    }

    /// Applies the post-claim capability checks and derives whether the UUID
    /// mode is effective.
    ///
    /// Returns whether the UUID is effective; the caller owns the
    /// capabilities probe and the persistence step.
    pub(super) fn validate_uuid_support(&self, uuid_mode: UuidMode) -> Result<bool> {
        if !self.can_report_directory_type() {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot report directory entry types"
            );
        }
        if !self.can_mknod_char() && !self.can_store_private_xattr() {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form"
            );
        }
        match uuid_mode {
            UuidMode::On => {
                if !self.can_store_private_xattr() {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the upper filesystem cannot persist the overlay uuid"
                    );
                }
                Ok(true)
            }
            UuidMode::Auto => Ok(self.can_store_private_xattr()),
            UuidMode::Off | UuidMode::Null => Ok(false),
        }
    }
}

/// A [`DirentVisitor`] that records whether any non-dot entry reports
/// `InodeType::Unknown`.
///
/// The `readdir_at` interface requires a visitor; no existing implementation
/// captures entry types.
struct DTypeProbeVisitor {
    saw_unknown_non_dot: bool,
}

impl DTypeProbeVisitor {
    fn new() -> Self {
        Self {
            saw_unknown_non_dot: false,
        }
    }
}

impl DirentVisitor for DTypeProbeVisitor {
    fn visit(&mut self, name: &str, _ino: u64, type_: InodeType, _offset: usize) -> Result<()> {
        if !is_dot_or_dotdot(name) && type_ == InodeType::Unknown {
            self.saw_unknown_non_dot = true;
        }
        Ok(())
    }
}
