// SPDX-License-Identifier: MPL-2.0

use core::mem;

use aster_rights::Full;
use ostd::mm::VmIo;

use crate::{prelude::*, vm::vmar::Vmar};
mod iovec;
pub mod net;
pub mod random;

pub use iovec::{copy_iovs_from_user, IoVec};

/// Read bytes into the `dest` buffer
/// from the user space of the current process.
/// If successful,
/// the `dest` buffer is filled with exact `dest.len` bytes.
pub fn read_bytes_from_user(src: Vaddr, dest: &mut [u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_bytes(src, dest)?)
}

/// Read a value of `Pod` type
/// from the user space of the current process.
pub fn read_val_from_user<T: Pod>(src: Vaddr) -> Result<T> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.read_val(src)?)
}

/// Write bytes from the `src` buffer
/// to the user space of the current process. If successful,
/// the write length will be equal to `src.len`.
pub fn write_bytes_to_user(dest: Vaddr, src: &[u8]) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_bytes(dest, src)?)
}

/// Write `val` to the user space of the current process.
pub fn write_val_to_user<T: Pod>(dest: Vaddr, val: &T) -> Result<()> {
    let current = current!();
    let root_vmar = current.root_vmar();
    Ok(root_vmar.write_val(dest, val)?)
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
    let current = current!();
    let vmar = current.root_vmar();
    read_cstring_from_vmar(vmar, addr, max_len)
}

/// Read CString from `vmar`. If possible, use `read_cstring_from_user` instead.
pub fn read_cstring_from_vmar(vmar: &Vmar<Full>, addr: Vaddr, max_len: usize) -> Result<CString> {
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
