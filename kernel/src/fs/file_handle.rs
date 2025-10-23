// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

//! Opened File Handle

use ostd::io::IoMem;

use super::inode_handle::InodeHandle;
use crate::{
    fs::utils::{
        AccessMode, FallocMode, Inode, InodeMode, IoctlCmd, Metadata, OpenArgs, SeekFrom,
        StatusFlags,
    },
    net::socket::Socket,
    prelude::*,
    process::{signal::Pollable, Gid, Uid},
};

/// The basic operations defined on a file
pub trait FileLike: Pollable + Send + Sync + Any {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "the file is not valid for reading");
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "the file is not valid for writing");
    }

    /// Read at the given file offset.
    ///
    /// The file must be seekable to support `read_at`.
    /// Unlike [`read`], `read_at` will not change the file offset.
    ///
    /// [`read`]: FileLike::read
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "read_at is not supported");
    }

    /// Write at the given file offset.
    ///
    /// The file must be seekable to support `write_at`.
    /// Unlike [`write`], `write_at` will not change the file offset.
    /// If the file is append-only, the `offset` will be ignored.
    ///
    /// [`write`]: FileLike::write
    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "write_at is not supported");
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }

    /// Obtains the mappable object to map this file into the user address space.
    ///
    /// If this file has a corresponding mappable object of [`Mappable`],
    /// then it can be either an inode or an MMIO region.
    fn mappable(&self) -> Result<Mappable> {
        return_errno_with_message!(Errno::EINVAL, "the file is not mappable");
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "resize is not supported");
    }

    /// Get the metadata that describes this file.
    fn metadata(&self) -> Metadata;

    #[expect(dead_code)]
    fn mode(&self) -> Result<InodeMode> {
        return_errno_with_message!(Errno::EINVAL, "mode is not supported");
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "set_mode is not supported");
    }

    #[expect(dead_code)]
    fn owner(&self) -> Result<Uid> {
        return_errno_with_message!(Errno::EPERM, "owner is not supported");
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "set_owner is not supported");
    }

    #[expect(dead_code)]
    fn group(&self) -> Result<Gid> {
        return_errno_with_message!(Errno::EPERM, "group is not supported");
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "set_group is not supported");
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::empty()
    }

    fn set_status_flags(&self, _new_flags: StatusFlags) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "set_status_flags is not supported");
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported");
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "fallocate is not supported");
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        None
    }

    /// Convert this file to a pseudo file.
    ///
    /// If this file is not a pseudo file, returns `None`.
    fn into_pseudo(self: Arc<Self>) -> Option<Arc<dyn PseudoFile>> {
        None
    }

    fn inode(&self) -> Option<&Arc<dyn Inode>> {
        None
    }
}

/// A pseudo file that do not have a corresponding `Path`.
pub trait PseudoFile: FileLike {
    /// Opens the pseudo file with the given `OpenArgs`.
    ///
    /// Returns an `Arc` to the newly opened pseudo file.
    fn open(&self, open_args: OpenArgs) -> Result<Arc<dyn PseudoFile>> {
        return_errno_with_message!(Errno::EACCES, "open is not supported");
    }

    fn display_name(&self) -> String {
        // TODO: remove this default implementation after all pseudo files implement this method.
        String::from("[pseudo file]")
    }
}

impl dyn FileLike {
    pub fn downcast_ref<T: FileLike>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    pub fn read_bytes(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read(&mut writer)
    }

    pub fn write_bytes(&self, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write(&mut reader)
    }

    pub fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer)
    }

    #[expect(dead_code)]
    pub fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader)
    }

    pub fn as_socket_or_err(&self) -> Result<&dyn Socket> {
        self.as_socket()
            .ok_or_else(|| Error::with_message(Errno::ENOTSOCK, "the file is not a socket"))
    }

    pub fn as_inode_or_err(&self) -> Result<&InodeHandle> {
        self.downcast_ref().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the file is not related to an inode")
        })
    }
}

/// An object that may be memory mapped into the user address space.
#[derive(Debug, Clone)]
pub enum Mappable {
    /// An inode object.
    Inode(Arc<dyn Inode>),
    /// An MMIO region.
    #[expect(dead_code)]
    IoMem(IoMem),
}
