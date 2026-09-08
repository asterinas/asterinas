// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Upper-filesystem capability probes.
//!
//! After the upper/workdir pair is claimed, this module measures the upper
//! filesystem's capabilities — on the upper itself and on the workdir
//! workspace — used to decide whiteout and UUID support for a writable
//! overlay mount.

use super::{super::policy::UuidMode, inuse::OVERLAY_UUID_SIZE};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            inode::{OverlayRecordName, OverlayXattrPrefix, overlay_record_name},
            workdir_temp_name,
        },
        utils::DirentVisitor,
        vfs::{
            inode::{Inode, MknodType},
            path::{Path, is_dot_or_dotdot},
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

impl UpperFilesystemCapabilities {
    /// The probes run at mount time on the private workdir clone view: the
    /// private-xattr probe reads the upper root directly, while the d_type
    /// and char-device probes create and remove transient entries under the
    /// workspace `Path` (transient dcache entries are overlay-private).
    pub(super) fn probe(
        upper_inode: &Arc<dyn Inode>,
        workspace_path: &Path,
        prefix: OverlayXattrPrefix,
    ) -> Result<Self> {
        let can_store_private_xattr = Self::probe_private_xattr(upper_inode, prefix)?;
        let can_report_directory_type = Self::probe_d_type(workspace_path)?;
        let can_mknod_char = Self::probe_mknod_char(workspace_path)?;
        Ok(Self {
            can_store_private_xattr,
            can_report_directory_type,
            can_mknod_char,
        })
    }

    fn probe_private_xattr(
        upper_inode: &Arc<dyn Inode>,
        prefix: OverlayXattrPrefix,
    ) -> Result<bool> {
        let name = overlay_record_name(OverlayRecordName::Uuid, prefix)?;
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

    fn probe_d_type(workspace_path: &Path) -> Result<bool> {
        let d_type_probe_name = workdir_temp_name(D_TYPE_PROBE_PREFIX);
        workspace_path.new_child(&d_type_probe_name, InodeType::File, InodeMode::empty())?;
        let mut d_type_probe = DTypeProbeVisitor::new();
        let mut offset = 0;
        let d_type_scan_result = loop {
            match workspace_path.inode().readdir_at(offset, &mut d_type_probe) {
                Ok(0) => break Ok(()),
                Ok(visited) => offset += visited,
                Err(err) => break Err(err),
            }
        };
        match d_type_scan_result {
            Ok(()) => {
                workspace_path.unlink(&d_type_probe_name)?;
                Ok(!d_type_probe.saw_unknown_non_dot)
            }
            Err(err) => {
                let _ = workspace_path.unlink(&d_type_probe_name);
                Err(err)
            }
        }
    }

    fn probe_mknod_char(workspace_path: &Path) -> Result<bool> {
        let probe_name = workdir_temp_name(CHAR_DEVICE_PROBE_PREFIX);
        match workspace_path.mknod(&probe_name, InodeMode::empty(), MknodType::CharDevice(0)) {
            Ok(_) => {
                workspace_path.unlink(&probe_name)?;
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

    pub(in overlayfs) fn can_mknod_char(&self) -> bool {
        self.can_mknod_char
    }

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
