// SPDX-License-Identifier: MPL-2.0

use ostd::mm::Fallible;

use super::{MultiRead, VmReaderArray};
use crate::prelude::*;

/// A trait providing the ability to read a C string from the user space.
pub trait ReadCString {
    /// Reads a C string from `self`.
    ///
    /// This method should read the bytes iteratively in `self` until
    /// encountering the end of the reader or reading a `\0` (which is also
    /// included in the final C String).
    fn read_cstring(&mut self) -> Result<CString>;

    /// Reads a C string from `self` with a maximum length of `max_len`.
    ///
    /// This method functions similarly to [`ReadCString::read_cstring`],
    /// but imposes an additional limit on the length of the C string.
    fn read_cstring_with_max_len(&mut self, max_len: usize) -> Result<CString>;
}

impl ReadCString for VmReader<'_, Fallible> {
    fn read_cstring(&mut self) -> Result<CString> {
        self.read_cstring_with_max_len(self.remain())
    }

    fn read_cstring_with_max_len(&mut self, max_len: usize) -> Result<CString> {
        // This implementation is inspired by
        // the `do_strncpy_from_user` function in Linux kernel.
        // The original Linux implementation can be found at:
        // <https://elixir.bootlin.com/linux/v6.0.9/source/lib/strncpy_from_user.c#L28>
        let mut buffer: Vec<u8> = Vec::with_capacity(max_len);

        if read_until_nul_byte(self, &mut buffer, max_len)? {
            return Ok(CString::from_vec_with_nul(buffer).unwrap());
        }

        return_errno_with_message!(
            Errno::EFAULT,
            "no nul terminator is present before reaching the buffer limit"
        );
    }
}

impl ReadCString for VmReaderArray<'_> {
    fn read_cstring(&mut self) -> Result<CString> {
        self.read_cstring_with_max_len(self.sum_lens())
    }

    fn read_cstring_with_max_len(&mut self, max_len: usize) -> Result<CString> {
        let mut buffer: Vec<u8> = Vec::with_capacity(max_len);

        for reader in self.readers_mut() {
            if read_until_nul_byte(reader, &mut buffer, max_len)? {
                return Ok(CString::from_vec_with_nul(buffer).unwrap());
            }
        }

        return_errno_with_message!(
            Errno::EFAULT,
            "no nul terminator is present before reaching the buffer limit"
        );
    }
}

/// Reads bytes from `reader` into `buffer` until a nul byte is found.
///
/// This method returns the following values:
/// 1. `Ok(true)`: If a nul byte is found in the reader;
/// 2. `Ok(false)`: If no nul byte is found and the `reader` is exhausted;
/// 3. `Err(_)`: If an error occurs while reading from the `reader`.
fn read_until_nul_byte(
    reader: &mut VmReader,
    buffer: &mut Vec<u8>,
    max_len: usize,
) -> Result<bool> {
    macro_rules! read_one_byte_at_a_time_while {
        ($cond:expr) => {
            while $cond {
                let byte = reader.read_val::<u8>()?;
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
        };
    }

    // Handle the first few bytes to make `cur_addr` aligned with `size_of::<usize>`
    read_one_byte_at_a_time_while!(
        !is_addr_aligned(reader.cursor() as usize) && buffer.len() < max_len && reader.has_remain()
    );

    // Handle the rest of the bytes in bulk
    let mut cloned_reader = reader.clone();
    while (buffer.len() + size_of::<usize>()) <= max_len {
        let Ok(word) = cloned_reader.read_val::<usize>() else {
            break;
        };

        if has_zero(word) {
            for byte in word.to_ne_bytes() {
                reader.skip(1);
                buffer.push(byte);
                if byte == 0 {
                    return Ok(true);
                }
            }
            unreachable!("The branch should never be reached unless `has_zero` has bugs.")
        }

        reader.skip(size_of::<usize>());
        buffer.extend_from_slice(&word.to_ne_bytes());
    }

    // Handle the last few bytes that are not enough for a word
    read_one_byte_at_a_time_while!(buffer.len() < max_len && reader.has_remain());

    if buffer.len() >= max_len {
        return_errno_with_message!(
            Errno::EFAULT,
            "no nul terminator is present before exceeding the maximum length"
        );
    } else {
        Ok(false)
    }
}

/// Determines whether the value contains a zero byte.
///
/// This magic algorithm is from the Linux `has_zero` function:
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/asm-generic/word-at-a-time.h#L93>
const fn has_zero(value: usize) -> bool {
    const ONE_BITS: usize = usize::from_le_bytes([0x01; size_of::<usize>()]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; size_of::<usize>()]);

    value.wrapping_sub(ONE_BITS) & !value & HIGH_BITS != 0
}

/// Checks if the given address is aligned.
const fn is_addr_aligned(addr: usize) -> bool {
    (addr & (size_of::<usize>() - 1)) == 0
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    fn init_buffer(cstrs: &[CString]) -> Vec<u8> {
        let mut buffer = vec![255u8; 100];

        let mut writer = VmWriter::from(buffer.as_mut_slice());

        for cstr in cstrs {
            writer.write(&mut VmReader::from(cstr.as_bytes_with_nul()));
        }

        buffer
    }

    #[ktest]
    fn read_multiple_cstring() {
        let strs = {
            let str1 = CString::new("hello").unwrap();
            let str2 = CString::new("world!").unwrap();
            vec![str1, str2]
        };

        let buffer = init_buffer(&strs);

        let mut reader = VmReader::from(buffer.as_slice()).to_fallible();
        let read_str1 = reader.read_cstring().unwrap();
        assert_eq!(read_str1, strs[0]);
        let read_str2 = reader.read_cstring().unwrap();
        assert_eq!(read_str2, strs[1]);

        assert!(reader
            .read_cstring()
            .is_err_and(|err| err.error() == Errno::EFAULT));
    }

    #[ktest]
    fn read_cstring_from_multiread() {
        let strs = {
            let str1 = CString::new("hello").unwrap();
            let str2 = CString::new("world!").unwrap();
            let str3 = CString::new("asterinas").unwrap();
            vec![str1, str2, str3]
        };

        let buffer = init_buffer(&strs);

        let mut readers = {
            let reader1 = VmReader::from(&buffer[0..20]).to_fallible();
            let reader2 = VmReader::from(&buffer[20..40]).to_fallible();
            let reader3 = VmReader::from(&buffer[40..60]).to_fallible();
            VmReaderArray::new(vec![reader1, reader2, reader3].into_boxed_slice())
        };

        let multiread = &mut readers as &mut dyn MultiRead;
        let read_str1 = multiread.read_cstring().unwrap();
        assert_eq!(read_str1, strs[0]);
        let read_str2 = multiread.read_cstring().unwrap();
        assert_eq!(read_str2, strs[1]);
        let read_str3 = multiread.read_cstring().unwrap();
        assert_eq!(read_str3, strs[2]);

        assert!(multiread
            .read_cstring()
            .is_err_and(|err| err.error() == Errno::EFAULT));
    }
}
