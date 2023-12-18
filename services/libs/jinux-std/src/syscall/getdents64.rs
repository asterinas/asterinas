use crate::fs::{
    file_table::FileDescripter,
    inode_handle::InodeHandle,
    utils::{DirentVisitor, InodeType},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::write_bytes_to_user;
use core::marker::PhantomData;

use super::SyscallReturn;
use super::SYS_GETDENTS64;

pub fn sys_getdents64(
    fd: FileDescripter,
    buf_addr: Vaddr,
    buf_len: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETDENTS64);
    debug!(
        "fd = {}, buf_addr = 0x{:x}, buf_len = 0x{:x}",
        fd, buf_addr, buf_len
    );

    let file = {
        let current = current!();
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    if inode_handle.dentry().inode_type() != InodeType::Dir {
        return_errno!(Errno::ENOTDIR);
    }
    let mut buffer = vec![0u8; buf_len];
    let mut reader = DirentBufferReader::<Dirent64>::new(&mut buffer);
    let _ = inode_handle.readdir(&mut reader)?;
    let read_len = reader.read_len();
    write_bytes_to_user(buf_addr, &buffer[..read_len])?;
    Ok(SyscallReturn::Return(read_len as _))
}

/// The Buffered DirentReader to visit the dir entry.
/// The DirentSerializer T decides how to serialize the data.
struct DirentBufferReader<'a, T: DirentSerializer> {
    buffer: &'a mut [u8],
    read_len: usize,
    phantom: PhantomData<T>,
}

impl<'a, T: DirentSerializer> DirentBufferReader<'a, T> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            read_len: 0,
            phantom: PhantomData,
        }
    }

    pub fn read_len(&self) -> usize {
        self.read_len
    }
}

impl<'a, T: DirentSerializer> DirentVisitor for DirentBufferReader<'a, T> {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        let dirent_serializer = T::new(ino, offset as u64, type_, CString::new(name)?);
        if self.read_len >= self.buffer.len() {
            return_errno_with_message!(Errno::EINVAL, "buffer is too small");
        }
        dirent_serializer.serialize(&mut self.buffer[self.read_len..])?;
        self.read_len += dirent_serializer.len();
        Ok(())
    }
}

/// The DirentSerializer can decide how to serialize the data.
trait DirentSerializer {
    /// Create a DirentSerializer.
    fn new(ino: u64, offset: u64, type_: InodeType, name: CString) -> Self;
    /// Get the length of a directory entry.
    fn len(&self) -> usize;
    /// Try to serialize a directory entry into buffer.
    fn serialize(&self, buf: &mut [u8]) -> Result<()>;
}

#[derive(Debug)]
struct Dirent64 {
    inner: Dirent64Inner,
    name: CString,
}

#[repr(packed)]
#[derive(Debug, Clone, Copy)]
struct Dirent64Inner {
    d_ino: u64,
    d_off: u64,
    d_reclen: u16,
    d_type: u8,
}

impl DirentSerializer for Dirent64 {
    fn new(ino: u64, offset: u64, type_: InodeType, name: CString) -> Self {
        let d_reclen = {
            let len =
                core::mem::size_of::<Dirent64Inner>() + name.as_c_str().to_bytes_with_nul().len();
            align_up(len, 8) as u16
        };
        let d_type = DirentType::from(type_) as u8;
        Self {
            inner: Dirent64Inner {
                d_ino: ino,
                d_off: offset,
                d_reclen,
                d_type,
            },
            name,
        }
    }

    fn len(&self) -> usize {
        self.inner.d_reclen as usize
    }

    fn serialize(&self, buf: &mut [u8]) -> Result<()> {
        if self.len() > buf.len() {
            return_errno_with_message!(Errno::EINVAL, "buffer is too small");
        }

        let d_ino = self.inner.d_ino;
        let d_off = self.inner.d_off;
        let d_reclen = self.inner.d_reclen;
        let d_type = self.inner.d_type;
        let items: [&[u8]; 5] = [
            d_ino.as_bytes(),
            d_off.as_bytes(),
            d_reclen.as_bytes(),
            d_type.as_bytes(),
            self.name.as_c_str().to_bytes_with_nul(),
        ];
        let mut offset = 0;
        for item in items {
            buf[offset..offset + item.len()].copy_from_slice(item);
            offset += item.len();
        }
        Ok(())
    }
}

#[allow(non_camel_case_types)]
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum DirentType {
    DT_UNKNOWN = 0,
    DT_FIFO = 1,
    DT_CHR = 2,
    DT_DIR = 4,
    DT_BLK = 6,
    DT_REG = 8,
    DT_LNK = 10,
    DT_SOCK = 12,
    DT_WHT = 14,
}

impl From<InodeType> for DirentType {
    fn from(type_: InodeType) -> Self {
        match type_ {
            InodeType::File => DirentType::DT_REG,
            InodeType::Dir => DirentType::DT_DIR,
            InodeType::SymLink => DirentType::DT_LNK,
            InodeType::CharDevice => DirentType::DT_CHR,
            InodeType::BlockDevice => DirentType::DT_BLK,
            InodeType::Socket => DirentType::DT_SOCK,
            InodeType::NamedPipe => DirentType::DT_FIFO,
        }
    }
}

fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}
