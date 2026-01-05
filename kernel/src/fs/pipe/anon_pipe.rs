// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt::Display,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use inherit_methods_macro::inherit_methods;

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::FdFlags,
        path::RESERVED_MOUNT_ID,
        pipe::{Pipe, common::PipeHandle},
        pseudofs::{PipeFs, PseudoInode},
        utils::{
            AccessMode, CreationFlags, Extension, FileSystem, Inode, InodeIo, InodeMode, InodeType,
            Metadata, StatusFlags, mkmod,
        },
    },
    prelude::*,
    process::{
        Gid, Uid,
        signal::{PollHandle, Pollable},
    },
};

/// Creates a pair of connected pipe file handles with the default capacity.
pub fn new_file_pair() -> Result<(Arc<AnonPipeFile>, Arc<AnonPipeFile>)> {
    let pipe_inode = Arc::new(AnonPipeInode::new());
    let reader = AnonPipeFile::open(
        pipe_inode.clone(),
        AccessMode::O_RDONLY,
        StatusFlags::empty(),
    )?;
    let writer = AnonPipeFile::open(pipe_inode, AccessMode::O_WRONLY, StatusFlags::empty())?;

    Ok((Arc::new(reader), Arc::new(writer)))
}

/// An anonymous pipe file.
pub struct AnonPipeFile {
    /// The opened pipe handle. `None` if the file is opened as a path.
    handle: Option<Box<PipeHandle>>,
    pipe_inode: Arc<dyn Inode>,
    status_flags: AtomicU32,
}

impl AnonPipeFile {
    pub fn open(
        pipe_inode: Arc<AnonPipeInode>,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        check_status_flags(status_flags)?;

        let handle = if !status_flags.contains(StatusFlags::O_PATH) {
            let handle = pipe_inode.pipe.open_anon(access_mode, status_flags)?;
            Some(handle)
        } else {
            None
        };

        Ok(Self {
            handle,
            pipe_inode,
            status_flags: AtomicU32::new(status_flags.bits()),
        })
    }
}

impl Pollable for AnonPipeFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if let Some(handle) = &self.handle {
            handle.poll(mask, poller)
        } else {
            IoEvents::NVAL
        }
    }
}

impl FileLike for AnonPipeFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let Some(handle) = &self.handle else {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        };

        if !handle.access_mode().is_readable() {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }

        handle.read_at(0, writer, self.status_flags())
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        let Some(handle) = &self.handle else {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        };

        if !handle.access_mode().is_writable() {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }

        handle.write_at(0, reader, self.status_flags())
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_truncate(self.status_flags.load(Ordering::Relaxed))
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        check_status_flags(new_flags)?;

        self.status_flags.store(new_flags.bits(), Ordering::Relaxed);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        if let Some(handle) = &self.handle {
            handle.access_mode()
        } else {
            // The file is opened with `O_PATH`. We follow Linux to report `O_RDONLY` here.
            AccessMode::O_RDONLY
        }
    }

    fn inode(&self) -> &Arc<dyn Inode> {
        &self.pipe_inode
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        let mut flags = self.status_flags().bits() | self.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        Box::new(FdInfo {
            flags,
            ino: self.inode().ino(),
        })
    }
}

struct FdInfo {
    flags: u32,
    ino: u64,
}

impl Display for FdInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "pos:\t{}", 0)?;
        writeln!(f, "flags:\t0{:o}", self.flags)?;
        // TODO: This should be the mount ID of the pseudo filesystem.
        writeln!(f, "mnt_id:\t{}", RESERVED_MOUNT_ID)?;
        writeln!(f, "ino:\t{}", self.ino)
    }
}

fn check_status_flags(status_flags: StatusFlags) -> Result<()> {
    if status_flags.contains(StatusFlags::O_DIRECT) {
        // "O_DIRECT .. Older kernels that do not support this flag will indicate this via an
        // EINVAL error."
        //
        // See <https://man7.org/linux/man-pages/man2/pipe.2.html>.
        return_errno_with_message!(Errno::EINVAL, "the `O_DIRECT` flag is not supported");
    }

    // TODO: Setting most of the other flags will succeed on Linux, but their effects need to be
    // validated.

    Ok(())
}

/// An anonymous pipe inode.
pub struct AnonPipeInode {
    /// The underlying pipe backend.
    pipe: Pipe,
    pseudo_inode: PseudoInode,
}

impl AnonPipeInode {
    fn new() -> Self {
        let pipe = Pipe::new();

        let pseudo_inode = PipeFs::singleton().alloc_inode(
            InodeType::NamedPipe,
            mkmod!(u+rw),
            Uid::new_root(),
            Gid::new_root(),
        );

        Self { pipe, pseudo_inode }
    }
}

#[inherit_methods(from = "self.pseudo_inode")]
impl InodeIo for AnonPipeInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize>;
}

#[inherit_methods(from = "self.pseudo_inode")]
impl Inode for AnonPipeInode {
    fn size(&self) -> usize;
    fn resize(&self, _new_size: usize) -> Result<()>;
    fn metadata(&self) -> Metadata;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;
}
