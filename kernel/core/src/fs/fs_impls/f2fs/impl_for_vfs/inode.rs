// SPDX-License-Identifier: MPL-2.0

//! [`FileOps`] and [`Inode`] implementations for
//! the F2FS inode.

use core::time::Duration;

use super::super::{
    inode::{Inode as F2fsInode, TimestampKind},
    prelude::*,
};
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags, SyncMode},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FileOps, Inode, Metadata, RenameMode, SymbolicLink},
            path::Dentry,
        },
    },
    process::{Gid, Uid},
};

impl FileOps for F2fsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.inode_type() == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
        }
        self.read_data(offset, writer)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        self.write_from_reader(offset, reader)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        F2fsInode::readdir_at(self, offset, visitor)
    }
}

impl Inode for F2fsInode {
    fn size(&self) -> usize {
        self.file_size()
    }

    fn resize(&self, _self_dentry: &Dentry, new_size: usize) -> Result<()> {
        self.truncate_to(new_size)
    }

    fn metadata(&self) -> Result<Metadata> {
        self.to_metadata()
    }

    fn ino(&self) -> u64 {
        self.ino() as u64
    }

    fn type_(&self) -> InodeType {
        self.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.perm())
    }

    fn set_mode(&self, _self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        self.set_perm(mode)
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.uid())
    }

    fn set_owner(&self, _self_dentry: &Dentry, uid: Uid) -> Result<()> {
        self.set_uid(uid.into())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.gid())
    }

    fn set_group(&self, _self_dentry: &Dentry, gid: Gid) -> Result<()> {
        self.set_gid(gid.into())
    }

    fn atime(&self) -> Duration {
        F2fsInode::atime(self)
    }

    fn set_atime(&self, _self_dentry: &Dentry, time: Duration) {
        let _ = self.set_timestamp(TimestampKind::Atime, time);
    }

    fn mtime(&self) -> Duration {
        F2fsInode::mtime(self)
    }

    fn set_mtime(&self, _self_dentry: &Dentry, time: Duration) {
        let _ = self.set_timestamp(TimestampKind::Mtime, time);
    }

    fn ctime(&self) -> Duration {
        F2fsInode::ctime(self)
    }

    fn set_ctime(&self, _self_dentry: &Dentry, time: Duration) {
        let _ = self.set_timestamp(TimestampKind::Ctime, time);
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Ok(self.create_child(name, type_, mode, None)?)
    }

    fn create_symlink(
        &self,
        _self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Ok(self.create_child(name, InodeType::SymLink, mode, Some(target))?)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(F2fsInode::lookup(self, name)?)
    }

    fn unlink(&self, child_dentry: &Dentry) -> Result<()> {
        self.remove_child(&child_dentry.name(), false)
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        self.remove_child(&child_dentry.name(), true)
    }

    fn rename(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        _replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
    ) -> Result<()> {
        if mode == RenameMode::Exchange {
            return_errno_with_message!(Errno::EOPNOTSUPP, "RENAME_EXCHANGE is not supported");
        }
        let new_dir = new_dir_dentry
            .inode()
            .downcast_ref::<F2fsInode>()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the parent is not F2FS"))?;
        self.rename_child(
            &old_child_dentry.name(),
            new_dir,
            new_name,
            mode == RenameMode::Replace,
        )
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link_target()
    }

    fn sync(&self, _mode: SyncMode) -> Result<()> {
        // Every change is written through to the block device.
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        F2fsInode::fs(self).expect("the F2FS instance is gone") as Arc<dyn FileSystem>
    }

    fn extension(&self) -> &Extension {
        F2fsInode::extension(self)
    }
}
