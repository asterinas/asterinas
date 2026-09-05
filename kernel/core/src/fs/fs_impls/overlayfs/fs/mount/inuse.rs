// SPDX-License-Identifier: MPL-2.0

//! Upper/workdir exclusivity claims and the unified 64-bit overlay identity.
//!
//! This module implements the inode `Extension` runtime lease that carries
//! the claim. Each claimed root inode hosts an `OverlayInuseSlot`.
//! The non-zero unified [`Uuid`] value serves two roles:
//!
//! - claim token: the per-slot compare-and-swap (CAS) on the slot's owner
//!   token guards this value so only one overlay can hold the claim;
//! - persisted overlay UUID: when effective, the value is stored under the
//!   mount's selected private-prefix uuid record (`trusted.overlay.uuid` by
//!   default, `user.overlay.uuid` in `userxattr` mode) on the upper root.

#![short_vis_path::add(overlayfs)]

use alloc::boxed::ThinBox;
use core::sync::atomic::{AtomicU64, Ordering};

use super::super::policy::UuidMode;
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            inode::{OverlayXattrType, workdir_temp_name},
            layer::ensure_distinct_non_overlapping,
            real::read_child_names,
        },
        vfs::{
            path::{Dentry, Path},
            xattr::XattrNamespace,
        },
    },
    prelude::*,
};

pub(super) const OVERLAY_UUID_SIZE: usize = 8;

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

/// The inode-owned runtime claim slot used by overlayfs mounts.
struct OverlayInuseSlot {
    owner_token: AtomicU64,
}

impl OverlayInuseSlot {
    fn new() -> Self {
        Self {
            owner_token: AtomicU64::new(0),
        }
    }

    /// Claims this slot for a non-zero owner token.
    fn try_claim(&self, token: u64) -> Result<()> {
        if token == 0 {
            return_errno_with_message!(Errno::EINVAL, "the overlay inuse token must be non-zero");
        }
        self.owner_token
            .compare_exchange(0, token, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| Error::with_message(Errno::EBUSY, "the inode is already in use"))?;
        Ok(())
    }

    /// Releases this slot only when `token` still owns it.
    fn release(&self, token: u64) {
        let _ = self
            .owner_token
            .compare_exchange(token, 0, Ordering::Release, Ordering::Relaxed);
    }
}

/// Pins the claimed inode so the claim cannot be evicted while the lease is held.
#[derive(Debug)]
struct InuseGuard {
    dentry: Arc<Dentry>,
    token: Uuid,
}

impl InuseGuard {
    fn try_claim(dentry: &Arc<Dentry>, identity: Uuid) -> Result<Self> {
        let slot: &OverlayInuseSlot = dentry
            .inode()
            .extension()
            .group3()
            .call_once(|| ThinBox::new_unsize(OverlayInuseSlot::new()))
            .downcast_ref()
            .expect("the overlay claim extension group holds only the claim slot");
        slot.try_claim(identity.value())?;
        Ok(Self {
            dentry: dentry.clone(),
            token: identity,
        })
    }
}

impl Drop for InuseGuard {
    fn drop(&mut self) {
        let slot: &OverlayInuseSlot = self
            .dentry
            .inode()
            .extension()
            .group3()
            .get()
            .and_then(|boxed| boxed.downcast_ref())
            .expect("a claimed inode hosts the overlay claim slot");
        slot.release(self.token.value());
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
    /// Validates the workdir/upper pair: one underlying filesystem, one mount, distinct roots.
    pub(super) fn validate_pair(upper: &Path, workdir: &Path) -> Result<()> {
        if upper.metadata()?.container_dev_id != workdir.metadata()?.container_dev_id {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must be on the same underlying filesystem"
            );
        }
        if !Arc::ptr_eq(upper.mount_node(), workdir.mount_node()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "workdir and upperdir must reside under the same mount"
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
        upper_dentry: &Dentry,
        uuid_mode: UuidMode,
        namespace: XattrNamespace,
    ) -> Result<Uuid> {
        match uuid_mode {
            UuidMode::On => match Self::read_identity_from_upper(upper_dentry, namespace)? {
                Some(existing) => Ok(existing),
                None => Ok(Uuid::generate()),
            },
            UuidMode::Auto => match Self::read_identity_from_upper(upper_dentry, namespace) {
                Ok(Some(existing)) => Ok(existing),
                Ok(None) | Err(_) => Ok(Uuid::generate()),
            },
            UuidMode::Off | UuidMode::Null => Ok(Uuid::generate()),
        }
    }

    pub(super) fn claim(
        upper_dentry: &Arc<Dentry>,
        workdir_dentry: &Arc<Dentry>,
        identity: Uuid,
    ) -> Result<Self> {
        let upper = InuseGuard::try_claim(upper_dentry, identity)?;
        let workdir = match InuseGuard::try_claim(workdir_dentry, identity) {
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

    pub(super) fn upper_dentry(&self) -> &Dentry {
        &self.upper.dentry
    }

    pub(super) fn next_temp_name(&self, target: &str) -> String {
        workdir_temp_name(target, &self.next_temp)
    }

    pub(in overlayfs) fn next_temp_counter(&self) -> &AtomicU64 {
        &self.next_temp
    }

    pub(super) fn prepare_workdir(&mut self) -> Result<()> {
        const WORKDIR_NAME: &str = "work";
        const WORKDIR_MODE: InodeMode = InodeMode::from_bits_truncate(0o700);

        let workspace = {
            let workdir = self.workdir.dentry.as_dir_dentry_or_err()?;
            match workdir.lookup_child(WORKDIR_NAME) {
                Ok(residue) if residue.type_().is_directory() => {
                    self.remove_work_entries(&residue, 0)?;
                    workdir.rmdir(WORKDIR_NAME)?;
                }
                Ok(_) => workdir.unlink(WORKDIR_NAME)?,
                Err(err) if err.error() == Errno::ENOENT => {}
                Err(err) => return Err(err),
            }
            workdir.create_child(WORKDIR_NAME, || {
                workdir
                    .inode()
                    .create(&workdir, WORKDIR_NAME, InodeType::Dir, WORKDIR_MODE)
            })?
        };
        self.workspace = Some(workspace);
        Ok(())
    }

    fn remove_work_entries(&self, dir_dentry: &Arc<Dentry>, level: usize) -> Result<()> {
        const WORKDIR_CLEANUP_MAX_DEPTH: usize = 2;

        let names = read_child_names(dir_dentry.inode())?;
        let dir = dir_dentry.as_dir_dentry_or_err()?;
        for name in names {
            let child = dir.lookup_child(&name)?;
            if child.type_().is_directory() {
                if level < WORKDIR_CLEANUP_MAX_DEPTH {
                    self.remove_work_entries(&child, level + 1)?;
                }
                dir.rmdir(&name)?;
            } else {
                dir.unlink(&name)?;
            }
        }
        Ok(())
    }

    /// Persists the claimed overlay uuid on the upper root and returns it.
    pub(super) fn persist_identity(&self, namespace: XattrNamespace) -> Result<Uuid> {
        let value = self.workdir.token.value().to_le_bytes();
        OverlayXattrType::Uuid.set_value_on(
            &self.upper.dentry,
            namespace,
            Some(value.as_slice()),
        )?;
        Ok(self.workdir.token)
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
        upper_dentry: &Dentry,
        namespace: XattrNamespace,
    ) -> Result<Option<Uuid>> {
        let mut value = [0u8; OVERLAY_UUID_SIZE];
        match OverlayXattrType::Uuid.get_value_from(upper_dentry, namespace, &mut value) {
            Ok(written) if written == OVERLAY_UUID_SIZE => {
                Ok(Some(Uuid::try_new(u64::from_le_bytes(value))?))
            }
            Ok(_) => return_errno_with_message!(
                Errno::EINVAL,
                "the persisted overlay uuid must be 8 bytes long"
            ),
            Err(err) if err.error() == Errno::ENODATA => Ok(None),
            Err(err) => Err(err),
        }
    }
}
