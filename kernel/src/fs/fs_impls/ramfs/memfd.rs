// SPDX-License-Identifier: MPL-2.0

//! Memfd Implementation.

use alloc::format;
use core::time::Duration;

use align_ext::AlignExt;
use aster_block::bio::BioWaiter;
use aster_rights::Rights;
use inherit_methods_macro::inherit_methods;
use spin::Once;

use super::fs::RamInode;
use crate::{
    fs::{
        inode_handle::InodeHandle,
        path::{Mount, Path},
        tmpfs::TmpFs,
        utils::{
            AccessMode, CachePage, Extension, FallocMode, FileSystem, Inode, InodeIo, InodeMode,
            InodeType, Metadata, PageCacheBackend, StatusFlags, XattrName, XattrNamespace,
            XattrSetFlags, mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::{perms::VmPerms, vmo::Vmo},
};

/// Maximum file name length for `memfd_create`, excluding the final `\0` byte.
///
/// See <https://man7.org/linux/man-pages/man2/memfd_create.2.html>
pub const MAX_MEMFD_NAME_LEN: usize = 249;

pub struct MemfdInode {
    inode: RamInode,
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

    pub(self) fn name(&self) -> &str {
        &self.name
    }
}

#[inherit_methods(from = "self.inode")]
impl PageCacheBackend for MemfdInode {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    fn write_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter>;
    fn npages(&self) -> usize;
}

#[inherit_methods(from = "self.inode")]
impl InodeIo for MemfdInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize>;

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        let seals = self.seals.lock();
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE) {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against writing");
        }

        if seals.contains(FileSeals::F_SEAL_GROW) {
            // For a memfd sealed with `F_SEAL_GROW`, if a write that would grow the file occurs,
            // the entire write within the page containing the EOF is rejected. Writes before
            // the EOF page are not affected.
            //
            // For detailed explanation, please see:
            // <https://github.com/asterinas/asterinas/pull/2555#discussion_r2509179520>
            //
            // Reference:
            // <https://elixir.bootlin.com/linux/v6.16.5/source/mm/shmem.c#L3309-L3310>
            // <https://github.com/google/gvisor/blob/6db745970118635edec4c973f47df2363924d3a7/test/syscalls/linux/memfd.cc#L261-L280>
            let old_size = self.inode.size();
            let new_size = offset.saturating_add(reader.remain());
            if new_size > old_size {
                let eof_page = old_size.align_down(PAGE_SIZE);
                if offset >= eof_page {
                    return_errno_with_message!(Errno::EPERM, "the file is sealed against growing");
                } else {
                    reader.limit(eof_page - offset);
                }
            }
        }

        self.inode.write_at(offset, reader, status_flags)
    }
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
    fn extension(&self) -> &Extension;
    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()>;
    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize>;
    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize>;
    fn remove_xattr(&self, name: XattrName) -> Result<()>;

    fn resize(&self, new_size: usize) -> Result<()> {
        let seals = self.seals.lock();
        let old_size = self.inode.size();
        if seals.contains(FileSeals::F_SEAL_SHRINK) && new_size < old_size {
            return_errno_with_message!(Errno::EPERM, "the file is sealed against shrinking");
        }
        if seals.contains(FileSeals::F_SEAL_GROW) && new_size > old_size {
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
        MemfdTmpFs::singleton().clone()
    }
}

pub trait MemfdInodeHandle: Sized {
    fn new_memfd(name: String, memfd_flags: MemfdFlags) -> Result<Self>;
    fn add_seals(&self, new_seals: FileSeals) -> Result<()>;
    fn get_seals(&self) -> Result<FileSeals>;
}

impl MemfdInodeHandle for InodeHandle {
    fn new_memfd(name: String, memfd_flags: MemfdFlags) -> Result<Self> {
        if name.len() > MAX_MEMFD_NAME_LEN {
            return_errno_with_message!(Errno::EINVAL, "the memfd name is too long");
        }

        let memfd_inode = Arc::new_cyclic(|weak_self| {
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

        let path = MemfdTmpFs::new_path(memfd_inode);

        InodeHandle::new_unchecked_access(path, AccessMode::O_RDWR, StatusFlags::empty())
    }

    fn add_seals(&self, new_seals: FileSeals) -> Result<()> {
        let rights = self.rights();
        if rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        if !rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EPERM, "the file is not opened writable");
        }

        memfd_inode_or_err(self)?.add_seals(new_seals)
    }

    fn get_seals(&self) -> Result<FileSeals> {
        let rights = self.rights();
        if rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        Ok(memfd_inode_or_err(self)?.get_seals())
    }
}

fn memfd_inode_or_err(file: &InodeHandle) -> Result<&MemfdInode> {
    file.path()
        .inode()
        .downcast_ref::<MemfdInode>()
        .ok_or_else(|| {
            Error::with_message(
                Errno::EINVAL,
                "file seals can only be applied to memfd files",
            )
        })
}

struct MemfdTmpFs {
    _private: (),
}

impl MemfdTmpFs {
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/shmem.c#L3828-L3850>
    pub(self) fn singleton() -> &'static Arc<TmpFs> {
        static MEMFD_TMPFS: Once<Arc<TmpFs>> = Once::new();

        MEMFD_TMPFS.call_once(TmpFs::new)
    }

    pub(self) fn new_path(memfd_inode: Arc<MemfdInode>) -> Path {
        Path::new_pseudo(Self::mount_node().clone(), memfd_inode, |inode| {
            let memfd_inode = inode.downcast_ref::<MemfdInode>().unwrap();
            format!("/memfd:{}", memfd_inode.name())
        })
    }

    fn mount_node() -> &'static Arc<Mount> {
        static MEMFD_TMPFS_MOUNT: Once<Arc<Mount>> = Once::new();

        MEMFD_TMPFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()))
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
