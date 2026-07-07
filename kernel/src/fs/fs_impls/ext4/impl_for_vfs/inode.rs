// SPDX-License-Identifier: MPL-2.0

//! VFS `FileOps` and `Inode` trait implementations for the ext4 `Inode`.
//!
//! Translates VFS read requests into ext4-internal operations: data reads
//! through the page cache, attribute getters, `lookup`, `readdir`, and symlink
//! reads; special files open into their live kernel objects. This is a
//! read-only mount, so the write-side methods return `EROFS`.

use core::time::Duration;

use aster_block::BLOCK_SIZE;
use device_id::DeviceId;

use crate::{
    device,
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags},
        fs_impls::ext4::Inode as Ext4Inode,
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, HardLinkability, Inode, Metadata, MknodType,
                RenameMode, SymbolicLink,
            },
            xattr::{XattrName, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::PageCache,
};

impl FileOps for Ext4Inode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // Read-only mount: serve reads (including O_DIRECT) through the page
        // cache. Access-time updates are a write and are disabled.
        self.read_at(offset, writer)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // Read-only mount: all writes are refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: write is disabled");
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        // Read-only mount: no access-time update (a write).
        self.readdir_at(offset, visitor)
    }
}

impl Inode for Ext4Inode {
    fn size(&self) -> usize {
        self.size()
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        // Read-only mount: truncation/extension is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: resize is disabled");
    }

    fn metadata(&self) -> Metadata {
        let container_dev_id = self
            .fs()
            .map(|fs| fs.container_device_id())
            .unwrap_or_else(|_| DeviceId::null());
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
            self_dev_id: self.device_id().and_then(DeviceId::from_encoded_u64),
            birth_at: Some(self.crtime()),
        }
    }

    fn ino(&self) -> u64 {
        self.ino() as u64
    }

    fn type_(&self) -> InodeType {
        self.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.mode())
    }

    fn set_mode(&self, _mode: InodeMode) -> Result<()> {
        // Read-only mount: chmod is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: set_mode is disabled");
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new(self.uid()))
    }

    fn set_owner(&self, _uid: Uid) -> Result<()> {
        // Read-only mount: chown is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: set_owner is disabled");
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new(self.gid()))
    }

    fn set_group(&self, _gid: Gid) -> Result<()> {
        // Read-only mount: chgrp is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: set_group is disabled");
    }

    fn atime(&self) -> Duration {
        self.atime()
    }

    fn set_atime(&self, _time: Duration) {
        // Read-only mount: timestamp updates are silently ignored (no-op).
    }

    fn mtime(&self) -> Duration {
        self.mtime()
    }

    fn set_mtime(&self, _time: Duration) {
        // Read-only mount: timestamp updates are silently ignored (no-op).
    }

    fn ctime(&self) -> Duration {
        self.ctime()
    }

    fn set_ctime(&self, _time: Duration) {
        // Read-only mount: timestamp updates are silently ignored (no-op).
    }

    fn page_cache(&self) -> Option<PageCache> {
        self.page_cache()
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        // Read-only mount: namespace creation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: create is disabled");
    }

    fn create_tmpfile(
        &self,
        _mode: InodeMode,
        _hard_linkability: HardLinkability,
    ) -> Result<Arc<dyn Inode>> {
        // Read-only mount: unnamed file creation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: create_tmpfile is disabled");
    }

    fn mknod(&self, _name: &str, _mode: InodeMode, _type_: MknodType) -> Result<Arc<dyn Inode>> {
        // Read-only mount: device/FIFO/socket creation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: mknod is disabled");
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        match self.inode_type() {
            // Special files route to their live kernel objects, like ext2.
            inode_type @ (InodeType::BlockDevice | InodeType::CharDevice) => {
                let Some(device_id) = self.device_id().and_then(DeviceId::from_encoded_u64) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the device ID is invalid",
                    )));
                };
                let device_type = inode_type
                    .device_type()
                    .expect("BlockDevice and CharDevice always have a device type");
                let Some(device) = device::lookup(device_type, device_id) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the required device ID does not exist",
                    )));
                };
                Some(device.open())
            }
            InodeType::NamedPipe => {
                let pipe = self.pipe().expect("NamedPipe inode must have a pipe");
                Some(pipe.open_named(access_mode, status_flags))
            }
            // Read-only mount: directories fall back to the direct inode path
            // (the shutdown ioctl shim is a write-path feature and is cut).
            _ => None,
        }
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        // Read-only mount: hard-link creation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: link is disabled");
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        // Read-only mount: unlink is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: unlink is disabled");
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        // Read-only mount: directory removal is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: rmdir is disabled");
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name)?)
    }

    fn rename(
        &self,
        _old_name: &str,
        _target: &Arc<dyn Inode>,
        _new_name: &str,
        _mode: RenameMode,
    ) -> Result<()> {
        // Read-only mount: rename is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: rename is disabled");
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link().map(SymbolicLink::Plain)
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        // Read-only mount: symlink-target updates are refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: write_link is disabled");
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        // Read-only mount: preallocation and hole-punching are refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: fallocate is disabled");
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs().unwrap()
    }

    fn extension(&self) -> &Extension {
        self.extension()
    }

    fn set_xattr(
        &self,
        _name: XattrName,
        _value_reader: &mut VmReader,
        _flags: XattrSetFlags,
    ) -> Result<()> {
        // Read-only mount: extended-attribute mutation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: set_xattr is disabled");
    }

    fn remove_xattr(&self, _name: XattrName) -> Result<()> {
        // Read-only mount: extended-attribute mutation is refused.
        return_errno_with_message!(Errno::EROFS, "read-only ext4: remove_xattr is disabled");
    }
}

#[cfg(ktest)]
mod tests {
    use alloc::sync::Arc;

    use aster_block::BLOCK_SIZE;
    use ostd::{
        mm::{VmReader, VmWriter},
        prelude::*,
    };

    use crate::{
        fs::{
            file::{InodeType, StatusFlags},
            fs_impls::ext4::test_utils::{
                Ext4FixtureBuilder, make_dir_block, make_dir_inode, make_file_inode,
            },
            vfs::{
                file_system::{FileSystem, FsFlags},
                inode::{FallocMode, Inode},
                xattr::{XattrName, XattrSetFlags},
            },
        },
        prelude::{Errno, Result},
    };

    fn assert_erofs<T>(result: Result<T>) {
        let Err(err) = result else {
            panic!("write-side operation must fail on read-only ext4");
        };
        assert_eq!(err.error(), Errno::EROFS);
    }

    /// Drives a mounted ext4 filesystem entirely through the VFS traits:
    /// `FileSystem` for stat, and `Inode`/`FileOps` for type, lookup, metadata,
    /// and reads.
    #[ktest]
    fn mount_via_vfs_and_read() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        // A directory (ino 12) with an entry "data" pointing at a file (ino 11).
        let dir_block = 102u32;
        let block = make_dir_block(&[(2, ".", 2), (2, "..", 2), (11, "data", 1)]);
        f.write_data_block(dir_block, &block);
        f.write_raw_inode(12, &make_dir_inode(dir_block));
        let content = b"vfs read works";
        f.write_data_block(103, content);
        f.write_raw_inode(11, &make_file_inode(103, content.len() as u32));

        // `FileSystem` trait.
        let fs: Arc<dyn FileSystem> = f.ext4.clone();
        assert_eq!(fs.name(), "ext4");
        assert_eq!(fs.sb().bsize, 4096);

        // `Inode` trait via dynamic dispatch.
        let dir_dyn: Arc<dyn Inode> = f.ext4.read_inode(12).unwrap();
        assert_eq!(dir_dyn.type_(), InodeType::Dir);

        let file_dyn = dir_dyn.lookup("data").unwrap();
        assert_eq!(file_dyn.ino(), 11);
        assert_eq!(file_dyn.size(), content.len());
        assert_eq!(file_dyn.metadata().type_, InodeType::File);

        // `FileOps::read_at` through the trait object.
        let mut buf = [0u8; 64];
        let mut writer = VmWriter::from(&mut buf[..content.len()]).to_fallible();
        let read = file_dyn
            .read_at(0, &mut writer, StatusFlags::empty())
            .unwrap();
        assert_eq!(read, content.len());
        assert_eq!(&buf[..content.len()], content);
    }

    #[ktest]
    fn fs_declares_read_only() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let fs: Arc<dyn FileSystem> = f.ext4.clone();

        assert!(fs.flags().contains(FsFlags::RDONLY));
        assert!(FsFlags::from_bits_truncate(fs.sb().flags as u32).contains(FsFlags::RDONLY));
    }

    #[ktest]
    fn write_side_inode_ops_return_erofs() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let inode: Arc<dyn Inode> = f.ext4.read_inode(2).unwrap();

        assert_erofs(inode.fallocate(FallocMode::Allocate, 0, BLOCK_SIZE));

        let mut value_reader = VmReader::from(b"value".as_slice()).to_fallible();
        let xattr_name = XattrName::try_from_full_name("user.ext4").unwrap();
        assert_erofs(inode.set_xattr(xattr_name, &mut value_reader, XattrSetFlags::empty()));

        let xattr_name = XattrName::try_from_full_name("user.ext4").unwrap();
        assert_erofs(inode.remove_xattr(xattr_name));

        assert_erofs(inode.write_link("target"));
    }
}
