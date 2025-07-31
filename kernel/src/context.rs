// SPDX-License-Identifier: MPL-2.0

//! The context that can be accessed from the current task, thread or process.

use core::{cell::Ref, mem};

use aster_rights::Full;
use ostd::{
    mm::{Fallible, Infallible, PodAtomic, VmReader, VmWriter, MAX_USERSPACE_VADDR},
    task::Task,
};

use crate::{
    prelude::*,
    process::{
        posix_thread::{PosixThread, ThreadLocal},
        Process,
    },
    thread::Thread,
    util::{MultiRead, VmReaderArray},
    vm::vmar::Vmar,
};

/// The context that can be accessed from the current POSIX thread.
#[derive(Clone)]
pub struct Context<'a> {
    pub process: &'a Process,
    pub thread_local: &'a ThreadLocal,
    pub posix_thread: &'a PosixThread,
    pub thread: &'a Thread,
    pub task: &'a Task,
}

impl Context<'_> {
    /// Gets the userspace of the current task.
    pub fn user_space(&self) -> CurrentUserSpace {
        CurrentUserSpace(self.thread_local.root_vmar().borrow())
    }
}

/// The user's memory space of the current task.
///
/// It provides methods to read from or write to the user space efficiently.
pub struct CurrentUserSpace<'a>(Ref<'a, Option<Vmar<Full>>>);

/// Gets the [`CurrentUserSpace`] from the current task.
///
/// This is slower than [`Context::user_space`]. Don't use this getter
/// If you get the access to the [`Context`].
#[macro_export]
macro_rules! current_userspace {
    () => {{
        use $crate::{context::CurrentUserSpace, process::posix_thread::AsThreadLocal};
        CurrentUserSpace::new(
            ostd::task::Task::current()
                .unwrap()
                .as_thread_local()
                .unwrap(),
        )
    }};
}

impl<'a> CurrentUserSpace<'a> {
    /// Creates a new `CurrentUserSpace` from the current task.
    ///
    /// If you have access to a [`Context`], it is preferable to call [`Context::user_space`].
    ///
    /// Otherwise, you can use the `current_userspace` macro
    /// to obtain an instance of `CurrentUserSpace` if it will only be used once.
    pub fn new(thread_local: &'a ThreadLocal) -> Self {
        let vmar_ref = thread_local.root_vmar().borrow();
        Self(vmar_ref)
    }

    /// Returns the root `Vmar` of the current userspace.
    ///
    /// # Panics
    ///
    /// This method will panic if the current process has cleared its `Vmar`.
    pub fn root_vmar(&self) -> &Vmar<Full> {
        self.0.as_ref().unwrap()
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if the `vaddr` and `len` do not represent a user space memory range.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        Ok(self.root_vmar().vm_space().reader(vaddr, len)?)
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if the `vaddr` and `len` do not represent a user space memory range.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        Ok(self.root_vmar().vm_space().writer(vaddr, len)?)
    }

    /// Reads bytes into the destination `VmWriter` from the user space of the
    /// current process.
    ///
    /// If the reading is completely successful, returns `Ok`. Otherwise, it
    /// returns `Err`.
    ///
    /// If the destination `VmWriter` (`dest`) is empty, this function still
    /// checks if the current task and user space are available. If they are,
    /// it returns `Ok`.
    pub fn read_bytes(&self, src: Vaddr, dest: &mut VmWriter<'_, Infallible>) -> Result<()> {
        let copy_len = dest.avail();

        if copy_len > 0 {
            check_vaddr(src)?;
        }

        let mut user_reader = self.reader(src, copy_len)?;
        user_reader.read_fallible(dest).map_err(|err| err.0)?;
        Ok(())
    }

    /// Reads a value typed `Pod` from the user space of the current process.
    pub fn read_val<T: Pod>(&self, src: Vaddr) -> Result<T> {
        if core::mem::size_of::<T>() > 0 {
            check_vaddr(src)?;
        }

        let mut user_reader = self.reader(src, core::mem::size_of::<T>())?;
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
    pub fn write_bytes(&self, dest: Vaddr, src: &mut VmReader<'_, Infallible>) -> Result<()> {
        let copy_len = src.remain();

        if copy_len > 0 {
            check_vaddr(dest)?;
        }

        let mut user_writer = self.writer(dest, copy_len)?;
        user_writer.write_fallible(src).map_err(|err| err.0)?;
        Ok(())
    }

    /// Writes `val` to the user space of the current process.
    pub fn write_val<T: Pod>(&self, dest: Vaddr, val: &T) -> Result<()> {
        if core::mem::size_of::<T>() > 0 {
            check_vaddr(dest)?;
        }

        let mut user_writer = self.writer(dest, core::mem::size_of::<T>())?;
        Ok(user_writer.write_val(val)?)
    }

    /// Atomically loads a `PodAtomic` value with [`Ordering::Relaxed`] semantics.
    ///
    /// # Panics
    ///
    /// This method will panic if `vaddr` is not aligned on a `core::mem::align_of::<T>()`-byte
    /// boundary.
    ///
    /// [`Ordering::Relaxed`]: core::sync::atomic::Ordering::Relaxed
    pub fn atomic_load<T: PodAtomic>(&self, vaddr: Vaddr) -> Result<T> {
        check_vaddr(vaddr)?;

        let user_reader = self.reader(vaddr, core::mem::size_of::<T>())?;
        Ok(user_reader.atomic_load()?)
    }

    /// Atomically updates a `PodAtomic` value with [`Ordering::Relaxed`] semantics.
    ///
    /// This method internally uses an atomic compare-and-exchange operation.If the value changes
    /// concurrently, this method will retry so the operation may be performed multiple times.
    ///
    /// # Panics
    ///
    /// This method will panic if `vaddr` is not aligned on a `core::mem::align_of::<T>()`-byte
    /// boundary.
    ///
    /// [`Ordering::Relaxed`]: core::sync::atomic::Ordering::Relaxed
    pub fn atomic_update<T>(&self, vaddr: Vaddr, op: impl Fn(T) -> T) -> Result<T>
    where
        T: PodAtomic + Eq,
    {
        check_vaddr(vaddr)?;

        let user_reader = self.reader(vaddr, core::mem::size_of::<T>())?;
        let mut user_writer = self.writer(vaddr, core::mem::size_of::<T>())?;
        loop {
            match user_writer.atomic_update(&user_reader, &op)? {
                (old_val, true) => return Ok(old_val),
                (_, false) => continue,
            }
        }
    }

    /// Reads a C string from the user space of the current process.
    /// The length of the string should not exceed `max_len`,
    /// including the final `\0` byte.
    pub fn read_cstring(&self, vaddr: Vaddr, max_len: usize) -> Result<CString> {
        if max_len > 0 {
            check_vaddr(vaddr)?;
        }

        // If `vaddr` is within user address space, adjust `max_len`
        // to ensure `vaddr + max_len` does not exceed `MAX_USERSPACE_VADDR`.
        // If `vaddr` is outside user address space, `max_len` will be set to zero
        // and further call to `self.reader` will return `EFAULT` in this case.
        let max_len = MAX_USERSPACE_VADDR.saturating_sub(vaddr).min(max_len);

        let mut user_reader = self.reader(vaddr, max_len)?;
        user_reader.read_cstring()
    }
}

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
    while (buffer.len() + mem::size_of::<usize>()) <= max_len {
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
    const ONE_BITS: usize = usize::from_le_bytes([0x01; mem::size_of::<usize>()]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; mem::size_of::<usize>()]);

    value.wrapping_sub(ONE_BITS) & !value & HIGH_BITS != 0
}

/// Checks if the user space pointer is below the lowest userspace address.
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
            "the userspace address is too small",
        ))
    } else {
        Ok(())
    }
}

/// Checks if the given address is aligned.
const fn is_addr_aligned(addr: usize) -> bool {
    (addr & (mem::size_of::<usize>() - 1)) == 0
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
