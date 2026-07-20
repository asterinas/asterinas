// SPDX-License-Identifier: MPL-2.0

//! Inode metadata operations: mode, ownership, timestamps, and xattr.
//!
//! Ext4 inode metadata includes the file mode, user and group ownership,
//! timestamps, and extended attributes. Attribute mutations must update the
//! relevant change timestamps and mark the inode descriptor dirty so the
//! on-disk inode can be refreshed during writeback.

use device_id::DeviceId;

use super::{
    super::{prelude::*, utils},
    FilePerm, Inode, InodePayload, RAW_BLOCK_PTRS_LEN, disk,
};
use crate::{
    fs::{
        file::InodeMode,
        vfs::{
            inode::Metadata,
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    process::{Gid, Uid},
};

impl Inode {
    /// Returns the encoded device ID for special files, 0 otherwise.
    pub(in crate::fs::fs_impls::ext4) fn device_id(&self) -> u64 {
        match &self.inner.read().payload {
            InodePayload::Device { device_id } => *device_id,
            _ => 0,
        }
    }

    /// Sets the encoded device ID for special files and persists it.
    pub(in crate::fs::fs_impls::ext4) fn set_device_id(&self, device_id: u64) -> Result<()> {
        if self.type_ != InodeType::CharDevice && self.type_ != InodeType::BlockDevice {
            return_errno!(Errno::EINVAL);
        }
        let mut inner = self.inner.write();
        inner.payload = InodePayload::Device { device_id };
        // Unlike ext2, whose writeback re-encodes the payload every time, this
        // module's partial-RMW writeback takes `i_block` from `desc` for
        // inodes without a block manager — the encoding must land there too,
        // or it never reaches the disk.
        let mut block = [0u32; RAW_BLOCK_PTRS_LEN];
        disk::write_device_id(&mut block, device_id);
        inner.desc.set_raw_block(block);
        inner.set_ctime(utils::now());
        Ok(())
    }

    /// Returns the inode metadata snapshot for stat-like queries.
    pub(in crate::fs::fs_impls::ext4) fn metadata(&self) -> Metadata {
        let container_dev_id = self
            .fs()
            .map(|fs| fs.container_device_id())
            .unwrap_or_else(|_| DeviceId::null());
        let self_dev_id = if matches!(
            self.inode_type(),
            InodeType::CharDevice | InodeType::BlockDevice
        ) {
            DeviceId::from_encoded_u64(self.device_id())
        } else {
            None
        };
        Metadata {
            ino: self.ino() as u64,
            size: self.size(),
            optimal_block_size: BLOCK_SIZE,
            nr_sectors_allocated: self.sector_count() as usize,
            last_access_at: self.atime(),
            last_modify_at: self.mtime(),
            last_meta_change_at: self.ctime(),
            type_: self.inode_type(),
            mode: self.mode(),
            nr_hard_links: self.link_count() as usize,
            uid: Uid::new(self.uid()),
            gid: Gid::new(self.gid()),
            container_dev_id,
            self_dev_id,
            birth_at: Some(self.crtime()),
        }
    }

    /// Returns the inode type.
    pub(in crate::fs::fs_impls::ext4) fn inode_type(&self) -> InodeType {
        self.type_
    }

    pub(in crate::fs::fs_impls::ext4) fn perm(&self) -> FilePerm {
        self.inner.read().desc.perm()
    }

    /// Returns the permission bits as a VFS `InodeMode`.
    pub(in crate::fs::fs_impls::ext4) fn mode(&self) -> InodeMode {
        InodeMode::from_bits_truncate(self.perm().bits() as _)
    }

    /// Updates the permission bits (chmod) and bumps ctime. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_mode(&self, mode: InodeMode) {
        let mut inner = self.inner.write();
        inner
            .desc
            .set_perm(FilePerm::from_bits_truncate(mode.bits()));
        inner.desc.set_ctime(utils::now());
    }

    pub(in crate::fs::fs_impls::ext4) fn uid(&self) -> u32 {
        self.inner.read().desc.uid()
    }

    /// Updates the owning uid (chown) and bumps ctime. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_uid(&self, uid: u32) {
        let mut inner = self.inner.write();
        inner.desc.set_uid(uid);
        inner.desc.set_ctime(utils::now());
    }

    pub(in crate::fs::fs_impls::ext4) fn gid(&self) -> u32 {
        self.inner.read().desc.gid()
    }

    /// Updates the owning gid (chgrp) and bumps ctime. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_gid(&self, gid: u32) {
        let mut inner = self.inner.write();
        inner.desc.set_gid(gid);
        inner.desc.set_ctime(utils::now());
    }

    pub(in crate::fs::fs_impls::ext4) fn atime(&self) -> Duration {
        self.inner.read().desc.atime()
    }

    /// Sets the last-access time. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_atime(&self, time: Duration) {
        self.inner.write().desc.set_atime(time);
    }

    pub(in crate::fs::fs_impls::ext4) fn mtime(&self) -> Duration {
        self.inner.read().desc.mtime()
    }

    /// Sets the last-modification time. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_mtime(&self, time: Duration) {
        self.inner.write().desc.set_mtime(time);
    }

    pub(in crate::fs::fs_impls::ext4) fn ctime(&self) -> Duration {
        self.inner.read().desc.ctime()
    }

    /// Sets the last-metadata-change time. Persists on fsync.
    pub(in crate::fs::fs_impls::ext4) fn set_ctime(&self, time: Duration) {
        self.inner.write().desc.set_ctime(time);
    }

    pub(in crate::fs::fs_impls::ext4) fn crtime(&self) -> Duration {
        self.inner.read().desc.crtime()
    }

    /// Reads one extended-attribute value and writes it to `value_writer`.
    pub(in crate::fs::fs_impls::ext4) fn get_xattr(
        &self,
        name: XattrName,
        value_writer: &mut VmWriter,
    ) -> Result<usize> {
        let xattr = self.xattr.as_ref().ok_or(Error::with_message(
            Errno::ENODATA,
            "xattr not supported on this inode type",
        ))?;
        xattr.get_xattr(name, value_writer)
    }

    /// Lists extended-attribute names in one namespace and writes them to
    /// `list_writer`.
    pub(in crate::fs::fs_impls::ext4) fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        let Some(xattr) = self.xattr.as_ref() else {
            return Ok(0);
        };
        xattr.list_xattr(namespace, list_writer)
    }

    /// Creates or replaces one extended attribute.
    pub(in crate::fs::fs_impls::ext4) fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let xattr = self.xattr.as_ref().ok_or(Error::with_message(
            Errno::EPERM,
            "xattr not supported on this inode type",
        ))?;
        xattr.set_xattr(name, value_reader, flags)?;
        let new_bid = xattr.bid();

        let mut inner = self.inner.write();
        inner.set_file_acl(new_bid);
        inner.set_ctime(utils::now());
        Ok(())
    }

    /// Removes one extended attribute.
    pub(in crate::fs::fs_impls::ext4) fn remove_xattr(&self, name: XattrName) -> Result<()> {
        let xattr = self.xattr.as_ref().ok_or(Error::with_message(
            Errno::EPERM,
            "xattr not supported on this inode type",
        ))?;
        xattr.remove_xattr(name)?;
        let new_bid = xattr.bid();

        let mut inner = self.inner.write();
        inner.set_file_acl(new_bid);
        inner.set_ctime(utils::now());
        Ok(())
    }
}
