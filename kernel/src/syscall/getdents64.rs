// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file_table::{FileDesc, get_file_fast},
        utils::{DirentVisitor, InodeType},
    },
    prelude::*,
};

pub fn sys_getdents(
    fd: FileDesc,
    buf_addr: Vaddr,
    buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, buf_addr = 0x{:x}, buf_len = 0x{:x}",
        fd, buf_addr, buf_len
    );

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let inode_handle = file.as_inode_handle_or_err()?;
    if inode_handle.path().type_() != InodeType::Dir {
        return_errno!(Errno::ENOTDIR);
    }
    let user_space = ctx.user_space();
    let writer = user_space.writer(buf_addr, buf_len)?;
    let mut reader = DirentBufferReader::<Dirent>::new(writer); // Use the non-64-bit reader
    let _ = inode_handle.readdir(&mut reader)?;
    let read_len = reader.read_len();
    fs::notify::on_access(&file);
    Ok(SyscallReturn::Return(read_len as _))
}

pub fn sys_getdents64(
    fd: FileDesc,
    buf_addr: Vaddr,
    buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, buf_addr = 0x{:x}, buf_len = 0x{:x}",
        fd, buf_addr, buf_len
    );

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let inode_handle = file.as_inode_handle_or_err()?;
    if inode_handle.path().type_() != InodeType::Dir {
        return_errno!(Errno::ENOTDIR);
    }
    let user_space = ctx.user_space();
    let writer = user_space.writer(buf_addr, buf_len)?;
    let mut reader = DirentBufferReader::<Dirent64>::new(writer);
    let _ = inode_handle.readdir(&mut reader)?;
    let read_len = reader.read_len();
    fs::notify::on_access(&file);
    Ok(SyscallReturn::Return(read_len as _))
}

/// The DirentSerializer can decide how to serialize the data.
trait DirentSerializer {
    /// Create a DirentSerializer.
    fn new(ino: u64, offset: u64, type_: InodeType, name: CString) -> Self;
    /// Get the length of a directory entry.
    fn len(&self) -> usize;
    /// Try to serialize a directory entry into buffer.
    fn serialize(&self, writer: &mut VmWriter) -> Result<()>;
}

/// The Buffered DirentReader to visit the dir entry.
/// The DirentSerializer T decides how to serialize the data.
struct DirentBufferReader<'a, T: DirentSerializer> {
    writer: VmWriter<'a>,
    read_len: usize,
    phantom: PhantomData<T>,
}

impl<'a, T: DirentSerializer> DirentBufferReader<'a, T> {
    pub fn new(writer: VmWriter<'a>) -> Self {
        Self {
            writer,
            read_len: 0,
            phantom: PhantomData,
        }
    }

    pub fn read_len(&self) -> usize {
        self.read_len
    }
}

impl<T: DirentSerializer> DirentVisitor for DirentBufferReader<'_, T> {
    fn visit(&mut self, name: &str, ino: u64, type_: InodeType, offset: usize) -> Result<()> {
        let dirent_serializer = T::new(ino, offset as u64, type_, CString::new(name)?);
        let len = dirent_serializer.len();
        if self.writer.avail() < len {
            return_errno_with_message!(
                Errno::EINVAL,
                "the buffer is too small for the directory entry"
            );
        }

        dirent_serializer.serialize(&mut self.writer)?;
        self.read_len += len;

        Ok(())
    }
}

#[derive(Debug)]
struct Dirent {
    inner: DirentInner,
    name: CString,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
struct DirentInner {
    d_ino: u64,
    d_off: u64,
    d_reclen: u16,
}

impl DirentSerializer for Dirent {
    fn new(ino: u64, offset: u64, _type_: InodeType, name: CString) -> Self {
        let d_reclen = {
            let len = size_of::<DirentInner>() + name.as_c_str().to_bytes_with_nul().len();
            len.align_up(8) as u16
        };
        Self {
            inner: DirentInner {
                d_ino: ino,
                d_off: offset,
                d_reclen,
            },
            name,
        }
    }

    fn len(&self) -> usize {
        self.inner.d_reclen as usize
    }

    fn serialize(&self, writer: &mut VmWriter) -> Result<()> {
        writer.write_val(&self.inner)?;

        let name_bytes = self.name.as_c_str().to_bytes_with_nul();
        let mut reader = VmReader::from(name_bytes);
        writer.write_fallible(&mut reader)?;

        let written_len = size_of::<DirentInner>() + name_bytes.len();
        let padding_len = self.len() - written_len;
        writer.fill_zeros(padding_len)?;

        Ok(())
    }
}

#[derive(Debug)]
struct Dirent64 {
    inner: Dirent64Inner,
    name: CString,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
struct Dirent64Inner {
    d_ino: u64,
    d_off: u64,
    d_reclen: u16,
    d_type: u8,
}

impl DirentSerializer for Dirent64 {
    fn new(ino: u64, offset: u64, type_: InodeType, name: CString) -> Self {
        let d_reclen = {
            let len = size_of::<Dirent64Inner>() + name.as_c_str().to_bytes_with_nul().len();
            len.align_up(8) as u16
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

    fn serialize(&self, writer: &mut VmWriter) -> Result<()> {
        writer.write_val(&self.inner)?;

        let name_bytes = self.name.as_c_str().to_bytes_with_nul();
        let mut reader = VmReader::from(name_bytes);
        writer.write_fallible(&mut reader)?;

        let written_len = size_of::<Dirent64Inner>() + name_bytes.len();
        let padding_len = self.len() - written_len;
        writer.fill_zeros(padding_len)?;

        Ok(())
    }
}

#[expect(non_camel_case_types)]
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
    #[expect(dead_code)]
    DT_WHT = 14,
}

impl From<InodeType> for DirentType {
    fn from(type_: InodeType) -> Self {
        match type_ {
            InodeType::Unknown => DirentType::DT_UNKNOWN,
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
