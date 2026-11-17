// SPDX-License-Identifier: MPL-2.0

//! VFS inode trait implementations for the canonical EROFS inode.

use core::time::Duration;

use super::super::{
    inode::ErofsInode,
    ondisk::xattr::{ShortXattrNamespace, XattrEntry},
};
use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags, SyncMode},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, HardLinkability, Inode, Metadata, MknodType,
                RenameMode, SymbolicLink,
            },
            path::Dentry,
            xattr::{XATTR_LIST_MAX_LEN, XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::{Readahead, Vmo},
};

impl FileOps for ErofsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.read_at(offset, writer, status_flags, None)
    }

    fn read_at_ahead(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
        readahead: &Readahead,
    ) -> Result<usize> {
        self.read_at(offset, writer, status_flags, Some(readahead))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EROFS))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.readdir_at(offset, visitor)
    }
}

impl Inode for ErofsInode {
    fn size(&self) -> usize {
        self.vfs_metadata().size
    }

    fn resize(&self, _self_dentry: &Dentry, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn metadata(&self) -> Result<Metadata> {
        Ok(*self.vfs_metadata())
    }

    fn ino(&self) -> u64 {
        self.vfs_metadata().ino
    }

    fn type_(&self) -> InodeType {
        self.vfs_metadata().type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.vfs_metadata().mode)
    }

    fn set_mode(&self, _self_dentry: &Dentry, _mode: InodeMode) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.vfs_metadata().uid)
    }

    fn set_owner(&self, _self_dentry: &Dentry, _uid: Uid) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.vfs_metadata().gid)
    }

    fn set_group(&self, _self_dentry: &Dentry, _gid: Gid) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn atime(&self) -> Duration {
        self.vfs_metadata().last_access_at
    }

    fn set_atime(&self, _self_dentry: &Dentry, _time: Duration) {}

    fn mtime(&self) -> Duration {
        self.vfs_metadata().last_modify_at
    }

    fn set_mtime(&self, _self_dentry: &Dentry, _time: Duration) {}

    fn ctime(&self) -> Duration {
        self.vfs_metadata().last_meta_change_at
    }

    fn set_ctime(&self, _self_dentry: &Dentry, _time: Duration) {}

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.file_system()
            .expect("erofs inode must belong to a live filesystem")
    }

    fn extension(&self) -> &Extension {
        self.extension()
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.file_cache().map(|cache| cache.as_vmo().clone())
    }

    fn open(
        &self,
        _self_dentry: &Dentry,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        open_error(self.type_(), access_mode, status_flags).map(|errno| Err(Error::new(errno)))
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _type_: InodeType,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EROFS))
    }

    fn create_symlink(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _target: &str,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EROFS))
    }

    fn create_tmpfile(
        &self,
        _self_dentry: &Dentry,
        _mode: InodeMode,
        _hard_linkability: HardLinkability,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EROFS))
    }

    fn mknod(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _mode: InodeMode,
        _type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EROFS))
    }

    fn link(&self, _self_dentry: &Dentry, _old_dentry: &Dentry, _name: &str) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn unlink(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn rmdir(&self, _child_dentry: &Dentry) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn rename(
        &self,
        _old_child_dentry: &Dentry,
        _new_dir_dentry: &Dentry,
        _new_name: &str,
        _replaced_inode: Option<&Arc<dyn Inode>>,
        _mode: RenameMode,
    ) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        // Preserve VFS expiry errors before the internal type/name checks.
        let _fs = self.file_system().map_err(|_| Error::new(Errno::EIO))?;
        Ok(ErofsInode::lookup(self, name.as_bytes()).map_err(|error| error.into_kernel_error())?)
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link().map(SymbolicLink::Plain)
    }

    fn sync(&self, _mode: SyncMode) -> Result<()> {
        Ok(())
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn get_xattr(&self, name: XattrName, writer: &mut VmWriter) -> Result<usize> {
        let namespace = match name.namespace() {
            XattrNamespace::User => ShortXattrNamespace::User,
            XattrNamespace::Trusted => ShortXattrNamespace::Trusted,
            XattrNamespace::Security => ShortXattrNamespace::Security,
            XattrNamespace::System => match name.full_name() {
                "system.posix_acl_access" => ShortXattrNamespace::AccessAcl,
                "system.posix_acl_default" => ShortXattrNamespace::DefaultAcl,
                _ => return Err(Error::new(Errno::ENODATA)),
            },
        };
        // XattrName validated the namespace prefix; ACL cases matched the
        // complete name above and therefore have an empty suffix.
        let suffix = &name.full_name()[namespace.prefix().len()..];
        let entry = self
            .description()
            .xattrs()
            .find(namespace, suffix)
            .ok_or_else(|| Error::new(Errno::ENODATA))?;
        let value = entry.value();
        let capacity = writer.avail();
        if capacity == 0 || value.is_empty() {
            return Ok(value.len());
        }
        if capacity < value.len() {
            return Err(Error::new(Errno::ERANGE));
        }
        writer.write_fallible(&mut VmReader::from(value))?;
        Ok(value.len())
    }

    fn list_xattr(&self, namespace: XattrNamespace, writer: &mut VmWriter) -> Result<usize> {
        let entries = self.description().xattrs().entries();
        // listxattr uses Trusted as its privileged visibility selector, not as
        // an exact namespace filter (see syscall/listxattr.rs).
        let matches_fn = |entry: &&XattrEntry| {
            namespace == XattrNamespace::Trusted
                || vfs_xattr_namespace(entry.namespace()) == namespace
        };
        let mut total = 0usize;
        for entry in entries.iter().filter(matches_fn) {
            total = total
                .checked_add(entry.namespace().prefix().len())
                .and_then(|size| size.checked_add(entry.suffix().len()))
                .and_then(|size| size.checked_add(1))
                .ok_or_else(|| Error::new(Errno::EOVERFLOW))?;
        }
        isize::try_from(total).map_err(|_| Error::new(Errno::EOVERFLOW))?;
        let capacity = writer.avail();
        if capacity == 0 {
            return Ok(total);
        }
        // Validate the entire request before copying any prefix or name.
        if capacity < total {
            return Err(Error::new(if capacity >= XATTR_LIST_MAX_LEN {
                Errno::E2BIG
            } else {
                Errno::ERANGE
            }));
        }
        for entry in entries.iter().filter(matches_fn) {
            writer.write_fallible(&mut VmReader::from(entry.namespace().prefix().as_bytes()))?;
            writer.write_fallible(&mut VmReader::from(entry.suffix().as_bytes()))?;
            writer.write_fallible(&mut VmReader::from(&[0u8][..]))?;
        }
        Ok(total)
    }

    fn set_xattr(
        &self,
        _self_dentry: &Dentry,
        _name: XattrName,
        _value_reader: &mut VmReader,
        _flags: XattrSetFlags,
    ) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }

    fn remove_xattr(&self, _self_dentry: &Dentry, _name: XattrName) -> Result<()> {
        Err(Error::new(Errno::EROFS))
    }
}

/// Maps independent disk indexes to the existing VFS namespace contract.
fn vfs_xattr_namespace(namespace: ShortXattrNamespace) -> XattrNamespace {
    match namespace {
        ShortXattrNamespace::User => XattrNamespace::User,
        ShortXattrNamespace::Trusted => XattrNamespace::Trusted,
        ShortXattrNamespace::Security => XattrNamespace::Security,
        ShortXattrNamespace::AccessAcl | ShortXattrNamespace::DefaultAcl => XattrNamespace::System,
    }
}

/// Selects the first-version open policy without creating per-open state.
fn open_error(
    type_: InodeType,
    access_mode: AccessMode,
    status_flags: StatusFlags,
) -> Option<Errno> {
    if status_flags.contains(StatusFlags::O_PATH) {
        return None;
    }
    match type_ {
        InodeType::File => {
            if access_mode.is_writable() {
                Some(Errno::EROFS)
            } else if status_flags.contains(StatusFlags::O_DIRECT) {
                Some(Errno::EOPNOTSUPP)
            } else {
                None
            }
        }
        InodeType::Dir => access_mode.is_writable().then_some(Errno::EISDIR),
        InodeType::SymLink => Some(Errno::ELOOP),
        // These inode types retain metadata support, but EROFS does not
        // provide device, FIFO or socket runtime objects.
        InodeType::BlockDevice
        | InodeType::CharDevice
        | InodeType::NamedPipe
        | InodeType::Socket => Some(Errno::EOPNOTSUPP),
        // Checked EROFS metadata rejects Unknown before inode construction.
        InodeType::Unknown => Some(Errno::EINVAL),
    }
}
