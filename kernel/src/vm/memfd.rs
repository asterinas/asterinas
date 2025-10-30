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

use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        inode_handle::{do_fallocate_util, do_resize_util, do_seek_util},
        ramfs::{new_detached_inode_in_memfd, RamFs, RamInode},
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
    vm::vmo::Vmo,
};

/// Maximum file name length for `memfd_create`, excluding the final `\0` byte.
///
/// See <https://man7.org/linux/man-pages/man2/memfd_create.2.html>
pub const MAX_MEMFD_NAME_LEN: usize = 249;

pub struct MemfdInode {
    inode: RamInode,
    #[expect(dead_code)]
    name: String,
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
        self.inode.write_at(offset, reader)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.inode.resize(new_size)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.inode.set_mode(mode)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
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
            let ram_inode =
                new_detached_inode_in_memfd(weak_self, mode, Uid::new_root(), Gid::new_root());

            MemfdInode {
                inode: ram_inode,
                name,
            }
        });

        Ok(Self {
            memfd_inode,
            offset: Mutex::new(0),
            access_mode: AccessMode::O_RDWR,
            status_flags: AtomicU32::new(0),
        })
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
        do_resize_util(&self.memfd_inode, self.status_flags(), new_size)
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
        do_seek_util(&self.memfd_inode, &self.offset, pos)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        do_fallocate_util(&self.memfd_inode, self.status_flags(), mode, offset, len)
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
