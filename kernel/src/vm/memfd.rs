// SPDX-License-Identifier: MPL-2.0

//! Memfd Implementation.

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use inherit_methods_macro::inherit_methods;

use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        inode_handle::{do_fallocate_util, do_resize_util, do_seek_util},
        ramfs::new_detached_inode,
        utils::{
            chmod, mkmod, AccessMode, FallocMode, Inode, InodeMode, IoctlCmd, Metadata, SeekFrom,
            StatusFlags,
        },
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
};

/// Maximum file name length for `memfd_create`, excluding the final `\0` byte.
///
/// See <https://man7.org/linux/man-pages/man2/memfd_create.2.html>
pub const MAX_MEMFD_NAME_LEN: usize = 249;

pub struct MemfdFile {
    inode: Arc<dyn Inode>,
    #[expect(dead_code)]
    name: String,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: AtomicU32,
    seals: Mutex<FileSeals>,
}

impl MemfdFile {
    pub fn new(name: &str, memfd_flags: MemfdFlags) -> Result<Self> {
        if name.len() > MAX_MEMFD_NAME_LEN {
            return_errno_with_message!(Errno::EINVAL, "MemfdManager: `name` is too long.");
        }

        // When Linux performs `memfd_create`, it first creates a RAM inode in a ramfs,
        // then immediately unlinks it, and finally returns only the file descriptor.
        // Therefore, when using `readlink("/proc/<pid>/fd/<fd>", ...)` to get the file
        // path of a `memfd` file, the result will have a `(deleted)` suffix. We stay
        // consistent with Linux here.
        //
        // Reference: <https://github.com/torvalds/linux/blob/379f604cc3dc2c865dc2b13d81faa166b6df59ec/mm/shmem.c#L5803-L5837>
        let name = format!("/memfd:{} (deleted)", name);

        let (allow_sealing, executable) = if memfd_flags.contains(MemfdFlags::MFD_NOEXEC_SEAL) {
            (true, false)
        } else {
            (memfd_flags.contains(MemfdFlags::MFD_ALLOW_SEALING), true)
        };

        let mut mode = mkmod!(a+rw);
        if executable {
            mode = chmod!(mode, a+x);
        }
        let inode = new_detached_inode(mode, Uid::new_root(), Gid::new_root());

        let mut seals = if allow_sealing {
            FileSeals::empty()
        } else {
            FileSeals::F_SEAL_SEAL
        };
        if !executable {
            seals |= FileSeals::F_SEAL_EXEC;
        }

        Ok(Self {
            inode,
            name,
            offset: Mutex::new(0),
            access_mode: AccessMode::O_RDWR,
            status_flags: AtomicU32::new(0),
            seals: Mutex::new(seals),
        })
    }

    pub fn seals(&self) -> &Mutex<FileSeals> {
        &self.seals
    }

    /// Checks whether writing is allowed to this memfd file.
    pub fn check_writable(&self) -> Result<()> {
        let seals = self.seals.lock();
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE) {
            return_errno_with_message!(Errno::EPERM, "File is sealed against writing");
        }
        Ok(())
    }
}

impl Pollable for MemfdFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT) & mask
    }
}

#[inherit_methods(from = "self.inode")]
impl FileLike for MemfdFile {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
    fn metadata(&self) -> Metadata;
    fn mode(&self) -> Result<InodeMode>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_EXEC)
            && (self.mode().unwrap() ^ mode).intersects(InodeMode::from_bits_truncate(0o111))
        {
            return_errno_with_message!(
                Errno::EPERM,
                "File is sealed against modifying executable bits"
            );
        }
        self.inode.set_mode(mode)
    }

    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut offset = self.offset.lock();

        let len = self.read_at(*offset, writer)?;
        *offset += len;

        Ok(len)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let mut offset = self.offset.lock();

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            *offset = self.inode.size();
        }

        let len = self.write_at(*offset, reader)?;
        *offset += len;

        Ok(len)
    }

    fn write_at(&self, mut offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !reader.has_remain() {
            return Ok(0);
        }

        let seals = self.seals.lock();
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE) {
            return_errno_with_message!(Errno::EPERM, "File is sealed against writing");
        }

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            // If the file has the O_APPEND flag, the offset is ignored
            offset = self.inode.size();
        }

        if seals.contains(FileSeals::F_SEAL_GROW) {
            let file_size = self.inode.size();
            if offset >= file_size {
                return_errno_with_message!(Errno::EPERM, "File is sealed against growing");
            } else {
                reader.limit(file_size - offset);
            }
        }

        self.inode.write_at(offset, reader)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_SHRINK) && new_size < self.inode.size() {
            return_errno_with_message!(Errno::EPERM, "File is sealed against shrinking");
        }
        if seals.contains(FileSeals::F_SEAL_GROW) && new_size > self.inode.size() {
            return_errno_with_message!(Errno::EPERM, "File is sealed against growing");
        }
        do_resize_util(&self.inode, self.status_flags(), new_size)
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
        do_seek_util(&self.inode, &self.offset, pos)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        let seals = self.seals.lock();
        if seals.contains(FileSeals::F_SEAL_GROW)
            && offset.checked_add(len).ok_or(Error::new(Errno::EINVAL))? > self.inode.size()
        {
            return_errno_with_message!(Errno::EPERM, "File is sealed against growing");
        }
        if seals.intersects(FileSeals::F_SEAL_WRITE | FileSeals::F_SEAL_FUTURE_WRITE)
            && mode == FallocMode::PunchHoleKeepSize
        {
            return_errno_with_message!(Errno::EPERM, "File is sealed against writing");
        }
        do_fallocate_util(&self.inode, self.status_flags(), mode, offset, len)
    }

    fn mappable(&self) -> Result<Mappable> {
        Ok(Mappable::Inode(self.inode.clone()))
    }

    fn inode(&self) -> Option<&Arc<dyn Inode>> {
        Some(&self.inode)
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
