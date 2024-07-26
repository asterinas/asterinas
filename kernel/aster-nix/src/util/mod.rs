// SPDX-License-Identifier: MPL-2.0

use core::mem;

use aster_rights::Full;
use ostd::{
    mm::{KernelSpace, VmIo, VmReader, VmWriter},
    task::current_task,
};

use crate::{prelude::*, vm::vmar::Vmar};
mod iovec;
pub mod net;
pub mod random;

pub use iovec::{copy_iovs_from_user, IoVec};

/// Reads bytes into the destination `VmWriter` from the user space of the
/// current process.
///
/// If the reading is completely successful, returns `Ok`. Otherwise, it
/// returns `Err`.
///
/// If the destination `VmWriter` (`dest`) is empty, this function still
/// checks if the current task and user space are available. If they are,
/// it returns `Ok`.
///
/// TODO: this API can be discarded and replaced with the API of `VmReader`
/// after replacing all related `buf` usages.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut VmWriter<'_>) -> Result<()> {
    let copy_len = dest.avail();

    if copy_len > 0 {
        check_vaddr(src)?;
    }

    let current_task = current_task().ok_or(Error::with_message(
        Errno::EFAULT,
        "the current task is missing",
    ))?;
    let user_space = current_task.user_space().ok_or(Error::with_message(
        Errno::EFAULT,
        "the user space is missing",
    ))?;

    let mut user_reader = user_space.vm_space().reader(src, copy_len)?;
    user_reader.read_fallible(dest).map_err(|err| err.0)?;
    Ok(())
}

/// Reads a value typed `Pod` from the user space of the current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> Result<T> {
    if core::mem::size_of::<T>() > 0 {
        check_vaddr(src)?;
    }

    let current_task = current_task().ok_or(Error::with_message(
        Errno::EFAULT,
        "the current task is missing",
    ))?;
    let user_space = current_task.user_space().ok_or(Error::with_message(
        Errno::EFAULT,
        "the user space is missing",
    ))?;

    let mut user_reader = user_space
        .vm_space()
        .reader(src, core::mem::size_of::<T>())?;
    Ok(user_reader.read_val()?)
}

/// Writes bytes from the source `VmReader` to the user space of the current
/// process.
///
/// If the writing is completely successful, returns `Ok`. Otherwise, it
/// returns `Err`.
///
/// If the source `VmReader` (`src`) is empty, this function still checks if
/// the current task and user space are available. If they are, it returns
/// `Ok`.
///
/// TODO: this API can be discarded and replaced with the API of `VmWriter`
/// after replacing all related `buf` usages.
pub fn write_bytes_to_user(dest: Vaddr, src: &mut VmReader<'_, KernelSpace>) -> Result<()> {
    let copy_len = src.remain();

    if copy_len > 0 {
        check_vaddr(dest)?;
    }

    let current_task = current_task().ok_or(Error::with_message(
        Errno::EFAULT,
        "the current task is missing",
    ))?;
    let user_space = current_task.user_space().ok_or(Error::with_message(
        Errno::EFAULT,
        "the user space is missing",
    ))?;

    let mut user_writer = user_space.vm_space().writer(dest, copy_len)?;
    user_writer.write_fallible(src).map_err(|err| err.0)?;
    Ok(())
}

/// Writes `val` to the user space of the current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) -> Result<()> {
    if core::mem::size_of::<T>() > 0 {
        check_vaddr(dest)?;
    }

    let current_task = current_task().ok_or(Error::with_message(
        Errno::EFAULT,
        "the current task is missing",
    ))?;
    let user_space = current_task.user_space().ok_or(Error::with_message(
        Errno::EFAULT,
        "the user space is missing",
    ))?;

    let mut user_writer = user_space
        .vm_space()
        .writer(dest, core::mem::size_of::<T>())?;
    Ok(user_writer.write_val(val)?)
}

/// Read a C string from the user space of the current process.
/// The length of the string should not exceed `max_len`,
/// including the final `\0` byte.
///
/// This implementation is inspired by
/// the `do_strncpy_from_user` function in Linux kernel.
/// The original Linux implementation can be found at:
/// <https://elixir.bootlin.com/linux/v6.0.9/source/lib/strncpy_from_user.c#L28>
pub fn read_cstring_from_user(addr: Vaddr, max_len: usize) -> Result<CString> {
    if max_len > 0 {
        check_vaddr(addr)?;
    }

    let current = current!();
    let vmar = current.root_vmar();
    read_cstring_from_vmar(vmar, addr, max_len)
}

/// Read CString from `vmar`. If possible, use `read_cstring_from_user` instead.
pub fn read_cstring_from_vmar(vmar: &Vmar<Full>, addr: Vaddr, max_len: usize) -> Result<CString> {
    if max_len > 0 {
        check_vaddr(addr)?;
    }

    let mut buffer: Vec<u8> = Vec::with_capacity(max_len);
    let mut cur_addr = addr;

    macro_rules! read_one_byte_at_a_time_while {
        ($cond:expr) => {
            while $cond {
                let byte = vmar.read_val::<u8>(cur_addr)?;
                buffer.push(byte);
                if byte == 0 {
                    return Ok(CString::from_vec_with_nul(buffer)
                        .expect("We provided 0 but no 0 is found"));
                }
                cur_addr += mem::size_of::<u8>();
            }
        };
    }

    // Handle the first few bytes to make `cur_addr` aligned with `size_of::<usize>`
    read_one_byte_at_a_time_while!(
        cur_addr % mem::size_of::<usize>() != 0 && buffer.len() < max_len
    );

    // Handle the rest of the bytes in bulk
    while (buffer.len() + mem::size_of::<usize>()) <= max_len {
        let Ok(word) = vmar.read_val::<usize>(cur_addr) else {
            break;
        };

        if has_zero(word) {
            for byte in word.to_ne_bytes() {
                buffer.push(byte);
                if byte == 0 {
                    return Ok(CString::from_vec_with_nul(buffer)
                        .expect("We provided 0 but no 0 is found"));
                }
            }
            unreachable!("The branch should never be reached unless `has_zero` has bugs.")
        }

        buffer.extend_from_slice(&word.to_ne_bytes());

        cur_addr += mem::size_of::<usize>();
    }

    // Handle the last few bytes that are not enough for a word
    read_one_byte_at_a_time_while!(buffer.len() < max_len);

    // Maximum length exceeded before finding the null terminator
    return_errno_with_message!(Errno::EFAULT, "Fails to read CString from user");
}

/// Determine whether the value contains a zero byte.
///
/// This magic algorithm is from the Linux `has_zero` function:
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/asm-generic/word-at-a-time.h#L93>
const fn has_zero(value: usize) -> bool {
    const ONE_BITS: usize = usize::from_le_bytes([0x01; mem::size_of::<usize>()]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; mem::size_of::<usize>()]);

    value.wrapping_sub(ONE_BITS) & !value & HIGH_BITS != 0
}

/// Check if the user space pointer is below the lowest userspace address.
///
/// If a pointer is below the lowest userspace address, it is likely to be a
/// NULL pointer. Reading from or writing to a NULL pointer should trigger a
/// segmentation fault.
///
/// If it is not checked here, a kernel page fault will happen and we would
/// deny the access in the page fault handler either. It may save a page fault
/// in some occasions. More importantly, double page faults may not be handled
/// quite well on some platforms.
fn check_vaddr(va: Vaddr) -> Result<()> {
    if va < crate::vm::vmar::ROOT_VMAR_LOWEST_ADDR {
        Err(Error::with_message(
            Errno::EFAULT,
            "Bad user space pointer specified",
        ))
    } else {
        Ok(())
    }
}
