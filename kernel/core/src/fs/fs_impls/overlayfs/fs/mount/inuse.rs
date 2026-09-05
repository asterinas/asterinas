// SPDX-License-Identifier: MPL-2.0

//! Upper/workdir exclusivity claims and the unified 64-bit overlay identity.
//!
//! This module implements the inode `Extension` runtime lease that carries
//! the claim. Each claimed root inode hosts a VFS-owned `OverlayInuseSlot`.
//! The non-zero unified [`Uuid`] value serves two roles:
//!
//! - claim token: the per-slot compare-and-swap (CAS) on the slot's owner
//!   token guards this value so only one overlay can hold the claim;
//! - persisted overlay UUID: when effective, the value is stored under the
//!   mount's selected private-prefix uuid record (`trusted.overlay.uuid` by
//!   default, `user.overlay.uuid` in `userxattr` mode) on the upper root.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::AtomicU64;

use super::super::policy::UuidMode;
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            inode::{OverlayInode, OverlayRecordName, workdir_temp_name},
            layer::ensure_distinct_non_overlapping,
            read_child_names,
        },
        vfs::{
            inode::Inode,
            inode_ext::InodeExt,
            path::{Dentry, Path},
            xattr::{XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
};

pub(super) const OVERLAY_UUID_SIZE: usize = 8;

const WORKDIR_NAME: &str = "work";

const WORKDIR_MODE: InodeMode = InodeMode::from_bits_truncate(0o700);

const WORKDIR_CLEANUP_MAX_DEPTH: usize = 2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in super::super) struct Uuid(u64);

impl Uuid {
    fn try_new(value: u64) -> Result<Self> {
        if value == 0 {
            return_errno_with_message!(Errno::EINVAL, "the overlay uuid must be non-zero");
        }
        Ok(Self(value))
    }

    pub(in super::super) fn value(&self) -> u64 {
        self.0
    }

    pub(super) fn generate() -> Self {
        loop {
            let mut bytes = [0u8; OVERLAY_UUID_SIZE];
            crate::util::random::getrandom(&mut bytes);
            let value = u64::from_le_bytes(bytes);
            if let Ok(uuid) = Self::try_new(value) {
                return uuid;
            }
        }
    }
}

/// Pins the claimed inode so the claim cannot be evicted while the lease is held.
#[derive(Debug)]
struct InuseGuard {
    inode: Arc<dyn Inode>,
    dentry: Arc<Dentry>,
    token: Uuid,
}

impl InuseGuard {
    fn try_claim(inode: &Arc<dyn Inode>, dentry: &Arc<Dentry>, identity: Uuid) -> Result<Self> {
        inode.overlay_inuse_slot().try_claim(identity.value())?;
        Ok(Self {
            inode: inode.clone(),
            dentry: dentry.clone(),
            token: identity,
        })
    }
}

impl Drop for InuseGuard {
    fn drop(&mut self) {
        self.inode.overlay_inuse_slot().release(self.token.value());
    }
}

#[derive(Debug)]
pub(in overlayfs) struct UpperWorkdirInuse {
    workdir: InuseGuard,
    upper: InuseGuard,
    workspace: Option<Arc<Dentry>>,
    next_temp: AtomicU64,
}

impl UpperWorkdirInuse {
    pub(super) fn validate_pair(upper: &Path, workdir: &Path) -> Result<()> {
        if upper.metadata()?.container_dev_id != workdir.metadata()?.container_dev_id {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must be on the same underlying filesystem"
            );
        }
        ensure_distinct_non_overlapping(
            upper.dentry(),
            workdir.dentry(),
            "workdir must be distinct from upperdir",
            "workdir must not be an ancestor or descendant of upperdir",
        )
    }

    pub(super) fn determine_identity(
        upper_inode: &Arc<dyn Inode>,
        uuid_mode: UuidMode,
        namespace: XattrNamespace,
    ) -> Result<Uuid> {
        match uuid_mode {
            UuidMode::On => match Self::read_identity_from_upper(upper_inode, namespace)? {
                Some(existing) => Ok(existing),
                None => Ok(Uuid::generate()),
            },
            UuidMode::Auto => match Self::read_identity_from_upper(upper_inode, namespace) {
                Ok(Some(existing)) => Ok(existing),
                Ok(None) | Err(_) => Ok(Uuid::generate()),
            },
            UuidMode::Off | UuidMode::Null => Ok(Uuid::generate()),
        }
    }

    pub(super) fn claim(upper_path: &Path, workdir_path: &Path, identity: Uuid) -> Result<Self> {
        let upper = InuseGuard::try_claim(upper_path.inode(), upper_path.dentry(), identity)?;
        let workdir =
            match InuseGuard::try_claim(workdir_path.inode(), workdir_path.dentry(), identity) {
                Ok(workdir) => workdir,
                Err(err) => {
                    drop(upper);
                    return Err(err);
                }
            };
        Ok(Self {
            workdir,
            upper,
            workspace: None,
            next_temp: AtomicU64::new(0),
        })
    }

    pub(super) fn upper_inode(&self) -> &Arc<dyn Inode> {
        &self.upper.inode
    }

    pub(super) fn next_temp_name(&self, target: &str) -> String {
        workdir_temp_name(target, &self.next_temp)
    }

    pub(in overlayfs) fn next_temp_counter(&self) -> &AtomicU64 {
        &self.next_temp
    }

    pub(super) fn prepare_workdir(&mut self, workdir_path: &Path) -> Result<()> {
        // The prepared path must be the claimed workdir root pinned by this lease.
        debug_assert!(
            Arc::ptr_eq(&self.workdir.dentry, workdir_path.dentry()),
            "the prepared workdir path is not the claimed workdir root"
        );
        match workdir_path.lookup_child(WORKDIR_NAME) {
            Ok(residue) if residue.type_().is_directory() => {
                self.remove_work_entries(&residue, 0)?;
                workdir_path.rmdir(WORKDIR_NAME)?;
            }
            Ok(_) => {
                workdir_path.unlink(WORKDIR_NAME)?;
            }
            Err(err) if err.error() == Errno::ENOENT => {}
            Err(err) => return Err(err),
        }
        let workspace = workdir_path.new_child(WORKDIR_NAME, InodeType::Dir, WORKDIR_MODE)?;
        self.workspace = Some(workspace.dentry().clone());
        Ok(())
    }

    fn remove_work_entries(&self, dir_path: &Path, level: usize) -> Result<()> {
        let names = read_child_names(dir_path.inode())?;
        for name in names {
            let child = dir_path.lookup_child(&name)?;
            if child.type_().is_directory() {
                if level < WORKDIR_CLEANUP_MAX_DEPTH {
                    self.remove_work_entries(&child, level + 1)?;
                }
                dir_path.rmdir(&name)?;
            } else {
                dir_path.unlink(&name)?;
            }
        }
        Ok(())
    }

    pub(super) fn persist_identity(&self, namespace: XattrNamespace) -> Result<()> {
        let value = self.workdir.token.value().to_le_bytes();
        let mut reader = VmReader::from(value.as_slice()).to_fallible();
        OverlayInode::set_overlay_xattr(
            &self.upper.inode,
            &self.upper.dentry,
            OverlayRecordName::Uuid,
            namespace,
            &mut reader,
            XattrSetFlags::CREATE_OR_REPLACE,
        )
    }

    pub(in overlayfs) fn workdir_workspace(&self) -> Result<&Arc<Dentry>> {
        self.workspace.as_ref().ok_or_else(|| {
            Error::with_message(
                Errno::EROFS,
                "the overlay workdir workspace is not prepared",
            )
        })
    }

    fn read_identity_from_upper(
        upper_inode: &Arc<dyn Inode>,
        namespace: XattrNamespace,
    ) -> Result<Option<Uuid>> {
        let name = OverlayRecordName::Uuid.construct_xattr_name(namespace)?;
        let mut value = [0u8; OVERLAY_UUID_SIZE];
        let mut writer = VmWriter::from(value.as_mut_slice()).to_fallible();
        match upper_inode.get_xattr(name, &mut writer) {
            Ok(written) if written == OVERLAY_UUID_SIZE => {
                Ok(Some(Uuid::try_new(u64::from_le_bytes(value))?))
            }
            Ok(_) => return_errno_with_message!(
                Errno::EINVAL,
                "the persisted overlay uuid has a malformed value"
            ),
            Err(err) if err.error() == Errno::ENODATA => Ok(None),
            Err(err) => Err(err),
        }
    }
}
