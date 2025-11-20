// SPDX-License-Identifier: MPL-2.0

//! Memfd Implementation.

use alloc::format;
use core::{
    fmt::Display,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use align_ext::AlignExt;
use aster_block::bio::BioWaiter;
use aster_rights::Rights;
use inherit_methods_macro::inherit_methods;
use spin::Once;

use super::fs::RamInode;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        file_table::FdFlags,
        inode_handle::{do_fallocate_util, do_resize_util, do_seek_util},
        path::{RESERVED_MOUNT_ID, check_open_util},
        tmpfs::TmpFs,
        utils::{
            AccessMode, CachePage, CreationFlags, Extension, FallocMode, FileSystem, Inode,
            InodeIo, InodeMode, InodeType, Metadata, OpenArgs, PageCacheBackend, SeekFrom,
            StatusFlags, XattrName, XattrNamespace, XattrSetFlags, mkmod,
        },
    },
    prelude::*,
    process::{
        Gid, Uid,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
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

    pub fn name(&self) -> &str {
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
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/shmem.c#L3828-L3850>
        static MEMFD_TMPFS: Once<Arc<TmpFs>> = Once::new();
        MEMFD_TMPFS.call_once(TmpFs::new).clone()
    }
}

pub struct MemfdFile {
    memfd_inode: Arc<dyn Inode>,
    offset: Mutex<usize>,
    status_flags: AtomicU32,
    rights: Rights,
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
            status_flags: AtomicU32::new(0),
            rights: Rights::READ | Rights::WRITE,
        })
    }

    pub fn open(inode: Arc<MemfdInode>, open_args: OpenArgs) -> Result<Self> {
        let inode: Arc<dyn Inode> = inode;
        let status_flags = open_args.status_flags;
        let access_mode = open_args.access_mode;

        if !status_flags.contains(StatusFlags::O_PATH) {
            inode.check_permission(access_mode.into())?;
        }
        check_open_util(inode.as_ref(), &open_args)?;

        if open_args.creation_flags.contains(CreationFlags::O_TRUNC)
            && !status_flags.contains(StatusFlags::O_PATH)
        {
            inode.resize(0)?;
        }

        let rights = if status_flags.contains(StatusFlags::O_PATH) {
            Rights::empty()
        } else {
            access_mode.into()
        };

        Ok(Self {
            memfd_inode: inode,
            offset: Mutex::new(0),
            status_flags: AtomicU32::new(open_args.status_flags.bits()),
            rights,
        })
    }

    pub fn add_seals(&self, new_seals: FileSeals) -> Result<()> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EPERM, "the file is not opened writable");
        }

        self.memfd_inode().add_seals(new_seals)
    }

    pub fn get_seals(&self) -> Result<FileSeals> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        Ok(self.memfd_inode().get_seals())
    }

    fn memfd_inode(&self) -> &MemfdInode {
        self.memfd_inode.downcast_ref::<MemfdInode>().unwrap()
    }
}

impl Pollable for MemfdFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileLike for MemfdFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut offset = self.offset.lock();

        let len = self.read_at(*offset, writer)?;
        *offset += len;

        Ok(len)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if !self.rights.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        self.memfd_inode
            .read_at(offset, writer, self.status_flags())
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut offset = self.offset.lock();

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            *offset = self.memfd_inode.size();
        }

        let len = self.write_at(*offset, reader)?;
        *offset += len;

        Ok(len)
    }

    fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        let status_flags = self.status_flags();
        if status_flags.contains(StatusFlags::O_APPEND) {
            // If the file has the `O_APPEND` flag, the offset is ignored.
            // FIXME: `O_APPEND` should ensure that new content is appended even if another process
            // is writing to the file concurrently.
            offset = self.memfd_inode.size();
        }
        self.memfd_inode.write_at(offset, reader, status_flags)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EINVAL, "the file is not opened writable");
        }

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
        self.rights.into()
    }

    fn seek(&self, pos: SeekFrom) -> Result<usize> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        do_seek_util(&self.offset, pos, Some(self.memfd_inode.size()))
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if !self.rights.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        do_fallocate_util(
            self.memfd_inode.as_ref(),
            self.status_flags(),
            mode,
            offset,
            len,
        )
    }

    fn mappable(&self) -> Result<Mappable> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        Ok(Mappable::Inode(self.memfd_inode.clone()))
    }

    fn ioctl(&self, _raw_ioctl: RawIoctl) -> Result<i32> {
        if self.rights.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }

        return_errno_with_message!(Errno::ENOTTY, "ioctl is not supported");
    }

    fn inode(&self) -> &Arc<dyn Inode> {
        &self.memfd_inode
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<MemfdFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", *self.inner.offset.lock())?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", RESERVED_MOUNT_ID)?;
                writeln!(f, "ino:\t{}", self.inner.inode().ino())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
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
