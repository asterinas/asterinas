// SPDX-License-Identifier: MPL-2.0

//! Upper-filesystem capability probes.
//!
//! After the upper/workdir pair is claimed, this module measures the upper
//! filesystem's capabilities — on the upper itself and on the workdir
//! workspace — used to decide whiteout and UUID support for a writable
//! overlay mount.

#![short_vis_path::add(overlayfs)]

use super::{
    super::policy::UuidMode,
    inuse::{OVERLAY_UUID_SIZE, UpperWorkdirInuse},
};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::inode::OverlayRecordName,
        utils::DirentVisitor,
        vfs::{
            inode::{Inode, MknodType},
            path::is_dot_or_dotdot,
            xattr::XattrNamespace,
        },
    },
    prelude::*,
};

const CHAR_DEVICE_PROBE_PREFIX: &str = ".overlay-char-device-probe-";
const D_TYPE_PROBE_PREFIX: &str = ".overlay-dtype-probe-";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) struct UpperFilesystemCapabilities {
    can_store_private_xattr: bool,
    can_report_directory_type: bool,
    can_mknod_char: bool,
}

/// The whiteout form the upper can publish; `Unsupported` is valid but blocks new whiteouts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in overlayfs) enum WhiteoutCapability {
    CharDevice,
    Xattr,
    Unsupported,
}

impl UpperFilesystemCapabilities {
    /// Probes the upper's private-xattr, d_type, and char-device support at mount time.
    pub(super) fn probe(
        upper_workdir: &UpperWorkdirInuse,
        namespace: XattrNamespace,
    ) -> Result<Self> {
        let can_store_private_xattr =
            Self::probe_private_xattr(upper_workdir.upper_inode(), namespace)?;
        let can_report_directory_type = Self::probe_d_type(upper_workdir)?;
        let can_mknod_char = Self::probe_mknod_char(upper_workdir)?;
        Ok(Self {
            can_store_private_xattr,
            can_report_directory_type,
            can_mknod_char,
        })
    }

    fn probe_private_xattr(
        upper_inode: &Arc<dyn Inode>,
        namespace: XattrNamespace,
    ) -> Result<bool> {
        let name = OverlayRecordName::Uuid.construct_xattr_name(namespace)?;
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

    fn probe_d_type(upper_workdir: &UpperWorkdirInuse) -> Result<bool> {
        let workspace = upper_workdir.workdir_workspace()?;
        let workspace_dir = workspace.as_dir_dentry_or_err()?;
        let d_type_probe_name = upper_workdir.next_temp_name(D_TYPE_PROBE_PREFIX);
        workspace_dir.create_child(&d_type_probe_name, || {
            workspace_dir.inode().create(
                &workspace_dir,
                &d_type_probe_name,
                InodeType::File,
                InodeMode::empty(),
            )
        })?;
        let mut d_type_probe = DTypeProbeVisitor::new();
        let mut offset = 0;
        let d_type_scan_result = loop {
            match workspace.inode().readdir_at(offset, &mut d_type_probe) {
                Ok(0) => break Ok(()),
                Ok(visited) => offset += visited,
                Err(err) => break Err(err),
            }
        };
        match d_type_scan_result {
            Ok(()) => {
                workspace_dir.unlink(&d_type_probe_name)?;
                Ok(!d_type_probe.saw_unknown_non_dot)
            }
            Err(err) => {
                let _ = workspace_dir.unlink(&d_type_probe_name);
                Err(err)
            }
        }
    }

    fn probe_mknod_char(upper_workdir: &UpperWorkdirInuse) -> Result<bool> {
        let workspace = upper_workdir.workdir_workspace()?;
        let workspace_dir = workspace.as_dir_dentry_or_err()?;
        let probe_name = upper_workdir.next_temp_name(CHAR_DEVICE_PROBE_PREFIX);
        match workspace_dir.mknod(&probe_name, InodeMode::empty(), MknodType::CharDevice(0)) {
            Ok(_) => {
                workspace_dir.unlink(&probe_name)?;
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

    pub(in overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.can_store_private_xattr
    }

    fn can_report_directory_type(&self) -> bool {
        self.can_report_directory_type
    }

    fn can_mknod_char(&self) -> bool {
        self.can_mknod_char
    }

    /// Derives the upper's published whiteout form, preferring the char-device `0:0` form.
    pub(in overlayfs) fn whiteout_capability(&self) -> WhiteoutCapability {
        if self.can_mknod_char() {
            WhiteoutCapability::CharDevice
        } else if self.can_store_private_xattr() {
            WhiteoutCapability::Xattr
        } else {
            WhiteoutCapability::Unsupported
        }
    }

    pub(super) fn validate_required_capabilities(&self) -> Result<()> {
        if !self.can_report_directory_type() {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the upper filesystem cannot report directory entry types"
            );
        }
        Ok(())
    }

    pub(super) fn validate_uuid_support(&self, uuid_mode: UuidMode) -> Result<bool> {
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
