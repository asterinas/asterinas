// SPDX-License-Identifier: MPL-2.0

//! Memfd Implementation.

use alloc::format;
use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use aster_block::bio::BioWaiter;
use inherit_methods_macro::inherit_methods;
use spin::Once;

use super::fs::{RamFs, RamInode};
use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        inode_handle::{do_fallocate_util, do_resize_util, do_seek_util},
        utils::{
            chmod, mkmod, AccessMode, CachePage, Extension, FallocMode, FileSystem, Inode,
            InodeMode, InodeType, IoctlCmd, Metadata, PageCacheBackend, SeekFrom, StatusFlags,
            XattrName, XattrNamespace, XattrSetFlags,
        },
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
    vm::{perms::VmPerms, vmo::Vmo},
};

/// Maximum file name length for `memfd_create`, excluding the final `\0` byte.
///
/// See <https://man7.org/linux/man-pages/man2/memfd_create.2.html>
pub const MAX_MEMFD_NAME_LEN: usize = 249;

pub struct MemfdInode {
    inode: RamInode,
    #[expect(dead_code)]
    name: String,
    seals: Mutex<FileSeals>,
}

impl MemfdInode {
    pub(self) fn add_seals(&self, mut new_seals: FileSeals) -> Result<()> {
        let mut seals = self.seals.lock();

        if seals.contains(FileSeals::F_SEAL_SEAL) {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against sealing");
        }

        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/memfd.c#L262-L266>
        if new_seals.contains(FileSeals::F_SEAL_EXEC)
            && self.mode().unwrap().intersects(mkmod!(a+x))
        {
            new_seals |= FileSeals::F_SEAL_SHRINK
                | FileSeals::F_SEAL_GROW
                | FileSeals::F_SEAL_WRITE
                | FileSeals::F_SEAL_FUTURE_WRITE;
        }

        if new_seals.contains(FileSeals::F_SEAL_WRITE) {
            let page_cache = self.page_cache().unwrap();
            page_cache.writable_mapping_status().deny()?;
        }

        *seals |= new_seals;

        Ok(())
    }

    pub(self) fn get_seals(&self) -> FileSeals {
        *self.seals.lock()
    }

    /// Checks whether writing to this memfd inode is allowed.
    ///
    /// This method restricts the `may_perms` if needed.
    pub fn check_writable(&self, perms: VmPerms, may_perms: &mut VmPerms) -> Result<()> {
        let seals = self.seals.lock();
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE) {
            if perms.contains(VmPerms::WRITE) {
                return_errno_with_message!(Errno::EPERM, "the file is sealed against writing");
            }
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/memfd.c#L356>
            may_perms.remove(VmPerms::MAY_WRITE);
        }
        Ok(())
    }
}

#[inherit_methods(from = "self.inode")]
impl PageCacheBackend for MemfdInode {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    fn write_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    fn npages(&self) -> usize;
}

#[inherit_methods(from = "self.inode")]
impl Inode for MemfdInode {
    fn metadata(&self) -> Metadata;
    fn size(&self) -> usize;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn page_cache(&self) -> Option<Arc<Vmo>>;
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize>;
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
    fn extension(&self) -> Option<&Extension>;
    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()>;
    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize>;
    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize>;
    fn remove_xattr(&self, name: XattrName) -> Result<()>;

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        let seals = self.seals.lock();
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE) {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against writing");
        }

        if seals.contains(FileSeals::F_SEAL_GROW) {
            let file_size = self.inode.size();
            if offset >= file_size {
                return_errno_with_message!(Errno::EPERM, "the file is sealed against growing");
            } else {
                reader.limit(file_size - offset);
            }
        }

        self.inode.write_at(offset, reader)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_SHRINK) && new_size < self.inode.size() {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against shrinking");
        }
        if seals.contains(FileSeals::F_SEAL_GROW) && new_size > self.inode.size() {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against growing");
        }

        self.inode.resize(new_size)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_EXEC)
            && (self.mode().unwrap() ^ mode).intersects(mkmod!(a+x))
        {
            return_errno_with_message!(
                Errno::EPERM,
                "the file is sealed against modifying executable bits"
            );
        }

        self.inode.set_mode(mode)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_GROW) && offset + len > self.inode.size() {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against growing");
        }
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE)
            && mode == FallocMode::PunchHoleKeepSize
        {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against writing");
        }

        self.inode.fallocate(mode, offset, len)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        // FIXME: Implement `AnonInodeFs` properly and link memfd inodes to it.
        static ANON_INODE_FS: Once<Arc<RamFs>> = Once::new();
        ANON_INODE_FS.call_once(RamFs::new).clone()
    }
}

pub struct MemfdFile {
    memfd_inode: Arc<dyn Inode>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: AtomicU32,
}

impl MemfdFile {
    pub fn new(name: &str, memfd_flags: MemfdFlags) -> Result<Self> {
        if name.len() > MAX_MEMFD_NAME_LEN {
            return_errno_with_message!(Errno::EINVAL, "MemfdManager: `name` is too long.");
        }

        let name = format!("/memfd:{}", name);

        let (allow_sealing, executable) = if memfd_flags.contains(MemfdFlags::MFD_NOEXEC_SEAL) {
            (true, false)
        } else {
            (memfd_flags.contains(MemfdFlags::MFD_ALLOW_SEALING), true)
        };

        let mode = if executable {
            mkmod!(a+rwx)
        } else {
            mkmod!(a+rw)
        };

        let memfd_inode = Arc::new_cyclic(|weak_self| {
            let ram_inode = RamInode::new_file_detached_in_memfd(
                weak_self,
                mode,
                Uid::new_root(),
                Gid::new_root(),
            );

            let mut seals = FileSeals::empty();
            if !allow_sealing {
                seals |= FileSeals::F_SEAL_SEAL;
            }
            if !executable {
                seals |= FileSeals::F_SEAL_EXEC;
            }

            MemfdInode {
                inode: ram_inode,
                name,
                seals: Mutex::new(seals),
            }
        });

        Ok(Self {
            memfd_inode,
            offset: Mutex::new(0),
            access_mode: AccessMode::O_RDWR,
            status_flags: AtomicU32::new(0),
        })
    }

    pub fn add_seals(&self, new_seals: FileSeals) -> Result<()> {
        if !self.access_mode.is_writable() {
            return_errno_with_message!(Errno::EPERM, "the file is not opened writable");
        }
        self.memfd_inode().add_seals(new_seals)
    }

    pub fn get_seals(&self) -> FileSeals {
        self.memfd_inode().get_seals()
    }

    fn memfd_inode(&self) -> &MemfdInode {
        self.memfd_inode.downcast_ref::<MemfdInode>().unwrap()
    }
}

impl Pollable for MemfdFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.memfd_inode.poll(mask, poller)
    }
}

#[inherit_methods(from = "self.memfd_inode")]
impl FileLike for MemfdFile {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
    fn metadata(&self) -> Metadata;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;

    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut offset = self.offset.lock();

        let len = self.read_at(*offset, writer)?;
        *offset += len;

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut offset = self.offset.lock();

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            *offset = self.memfd_inode.size();
        }

        let len = self.write_at(*offset, reader)?;
        *offset += len;

        Ok(len)
    }

    fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if self.status_flags().contains(StatusFlags::O_APPEND) {
            // If the file has the O_APPEND flag, the offset is ignored
            offset = self.memfd_inode.size();
        }

        self.memfd_inode.write_at(offset, reader)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        do_resize_util(self.memfd_inode.as_ref(), self.status_flags(), new_size)
    }

    fn status_flags(&self) -> StatusFlags {
        let bits = self.status_flags.load(Ordering::Relaxed);
        StatusFlags::from_bits(bits).unwrap()
    }

    fn set_status_flags(&self, new_status_flags: StatusFlags) -> Result<()> {
        self.status_flags
            .store(new_status_flags.bits(), Ordering::Relaxed);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    fn seek(&self, pos: SeekFrom) -> Result<usize> {
        do_seek_util(self.memfd_inode.as_ref(), &self.offset, pos)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        do_fallocate_util(
            self.memfd_inode.as_ref(),
            self.status_flags(),
            mode,
            offset,
            len,
        )
    }

    fn mappable(&self) -> Result<Mappable> {
        Ok(Mappable::Inode(self.memfd_inode.clone()))
    }
}

bitflags! {
    pub struct MemfdFlags: u32 {
        /// Close on exec.
        const MFD_CLOEXEC = 1 << 0;
        /// Allow sealing operations on this file.
        const MFD_ALLOW_SEALING = 1 << 1;
        /// Create in the hugetlbfs.
        const MFD_HUGETLB = 1 << 2;
        /// Not executable and sealed to prevent changing to executable.
        const MFD_NOEXEC_SEAL = 1 << 3;
        /// Executable.
        const MFD_EXEC = 1 << 4;
    }
}

bitflags! {
    pub struct FileSeals: u32 {
        /// Prevent further seals from being set.
        const F_SEAL_SEAL = 0x0001;
        /// Prevent file from shrinking.
        const F_SEAL_SHRINK = 0x0002;
        /// Prevent file from growing.
        const F_SEAL_GROW = 0x0004;
        /// Prevent writes.
        const F_SEAL_WRITE = 0x0008;
        /// Prevent future writes while mapped.
        const F_SEAL_FUTURE_WRITE = 0x0010;
        /// Prevent chmod modifying exec bits.
        const F_SEAL_EXEC = 0x0020;
    }
}
