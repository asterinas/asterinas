// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    util::{copy_iovs_from_user, IoVec},
};

pub fn sys_readv(fd: FileDesc, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<SyscallReturn> {
    let res = do_sys_readv(fd, io_vec_ptr, io_vec_count)?;
    Ok(SyscallReturn::Return(res as _))
}

pub fn sys_preadv(
    fd: FileDesc,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
    offset: i64,
) -> Result<SyscallReturn> {
    let res = do_sys_preadv(fd, io_vec_ptr, io_vec_count, offset, RWFFlag::empty())?;
    Ok(SyscallReturn::Return(res as _))
}

pub fn sys_preadv2(
    fd: FileDesc,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
    offset: i64,
    flags: u32,
) -> Result<SyscallReturn> {
    let flags = match RWFFlag::from_bits(flags) {
        Some(flags) => flags,
        None => return_errno_with_message!(Errno::EINVAL, "invalid flags"),
    };
    let res = if offset == -1 {
        do_sys_readv(fd, io_vec_ptr, io_vec_count)?
    } else {
        do_sys_preadv(fd, io_vec_ptr, io_vec_count, offset, flags)?
    };
    Ok(SyscallReturn::Return(res as _))
}

fn do_sys_preadv(
    fd: FileDesc,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
    offset: i64,
    _flags: RWFFlag,
) -> Result<usize> {
    debug!(
        "preadv: fd = {}, io_vec_ptr = 0x{:x}, io_vec_counter = 0x{:x}, offset = 0x{:x}",
        fd, io_vec_ptr, io_vec_count, offset
    );

    if offset < 0 {
        return_errno_with_message!(Errno::EINVAL, "offset cannot be negative");
    }

    let file = {
        let current = current!();
        let filetable = current.file_table().lock();
        filetable.get_file(fd)?.clone()
    };

    if io_vec_count == 0 {
        return Ok(0);
    }

    // Calculate the total buffer length and check for overflow
    let total_len = io_vec_count
        .checked_mul(core::mem::size_of::<IoVec>())
        .and_then(|val| val.checked_add(offset as usize));
    if total_len.is_none() {
        return_errno_with_message!(Errno::EINVAL, "offset + io_vec_count overflow");
    }

    let mut total_len: usize = 0;
    let mut cur_offset = offset as usize;

    let io_vecs = copy_iovs_from_user(io_vec_ptr, io_vec_count)?;
    for io_vec in io_vecs.as_ref() {
        if io_vec.is_empty() {
            continue;
        }
        if total_len.checked_add(io_vec.len()).is_none()
            || total_len
                .checked_add(io_vec.len())
                .and_then(|sum| sum.checked_add(cur_offset))
                .is_none()
            || total_len
                .checked_add(io_vec.len())
                .and_then(|sum| sum.checked_add(cur_offset))
                .map(|sum| sum > isize::MAX as usize)
                .unwrap_or(false)
        {
            return_errno_with_message!(Errno::EINVAL, "Total length overflow");
        }

        let mut buffer = vec![0u8; io_vec.len()];

        // TODO: According to the man page
        // at <https://man7.org/linux/man-pages/man2/readv.2.html>,
        // readv must be atomic,
        // but the current implementation does not ensure atomicity.
        // A suitable fix would be to add a `readv` method for the `FileLike` trait,
        // allowing each subsystem to implement atomicity.
        let read_len = file.read_at(cur_offset, &mut buffer)?;
        io_vec.write_exact_to_user(&buffer)?;
        total_len += read_len;
        cur_offset += read_len;
        if read_len == 0 || read_len < buffer.len() {
            // End of file reached or no more data to read
            break;
        }
    }

    Ok(total_len)
}

fn do_sys_readv(fd: FileDesc, io_vec_ptr: Vaddr, io_vec_count: usize) -> Result<usize> {
    debug!(
        "fd = {}, io_vec_ptr = 0x{:x}, io_vec_counter = 0x{:x}",
        fd, io_vec_ptr, io_vec_count
    );

    let file = {
        let current = current!();
        let filetable = current.file_table().lock();
        filetable.get_file(fd)?.clone()
    };

    if io_vec_count == 0 {
        return Ok(0);
    }

    let mut total_len = 0;

    let io_vecs = copy_iovs_from_user(io_vec_ptr, io_vec_count)?;
    for io_vec in io_vecs.as_ref() {
        if io_vec.is_empty() {
            continue;
        }

        let mut buffer = vec![0u8; io_vec.len()];
        // TODO: According to the man page
        // at <https://man7.org/linux/man-pages/man2/readv.2.html>,
        // readv must be atomic,
        // but the current implementation does not ensure atomicity.
        // A suitable fix would be to add a `readv` method for the `FileLike` trait,
        // allowing each subsystem to implement atomicity.
        let read_len = file.read(&mut buffer)?;
        io_vec.write_exact_to_user(&buffer)?;
        total_len += read_len;
        if read_len == 0 || read_len < buffer.len() {
            // End of file reached or no more data to read
            break;
        }
    }

    Ok(total_len)
}

bitflags! {
    struct RWFFlag: u32 {
        const RWF_DSYNC = 0x00000001;
        const RWF_HIPRI = 0x00000002;
        const RWF_SYNC = 0x00000004;
        const RWF_NOWAIT = 0x00000008;
    }
}
