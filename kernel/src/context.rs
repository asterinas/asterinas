// SPDX-License-Identifier: MPL-2.0

//! The context that can be accessed from the current task, thread or process.

use core::mem;

use ostd::{
    mm::{Fallible, Infallible, VmReader, VmSpace, VmWriter},
    task::Task,
};

use crate::{
    prelude::*,
    process::{posix_thread::PosixThread, Process},
    thread::Thread,
};

/// The context that can be accessed from the current POSIX thread.
#[derive(Clone)]
pub struct Context<'a> {
    pub process: &'a Process,
    pub posix_thread: &'a PosixThread,
    pub thread: &'a Thread,
    pub task: &'a Task,
}

impl Context<'_> {
    /// Gets the userspace of the current task.
    pub fn user_space(&self) -> CurrentUserSpace {
        CurrentUserSpace::new(self.task)
    }
}

/// The user's memory space of the current task.
///
/// It provides methods to read from or write to the user space efficiently.
pub struct CurrentUserSpace<'a>(&'a VmSpace);

/// Gets the [`CurrentUserSpace`] from the current task.
///
/// This is slower than [`Context::user_space`]. Don't use this getter
/// If you get the access to the [`Context`].
#[macro_export]
macro_rules! current_userspace {
    () => {
        CurrentUserSpace::new(&ostd::task::Task::current().unwrap())
    };
}

impl<'a> CurrentUserSpace<'a> {
    /// Creates a new `CurrentUserSpace` from the specified task.
    ///
    /// This method is _not_ recommended for use, as it does not verify whether the provided
    /// `task` is the current task in release builds.
    ///
    /// If you have access to a [`Context`], it is preferable to call [`Context::user_space`].
    ///
    /// Otherwise, you can use the `current_userspace` macro
    /// to obtain an instance of `CurrentUserSpace` if it will only be used once.
    ///
    /// # Panics
    ///
    /// This method will panic in debug builds if the specified `task` is not the current task.
    pub fn new(task: &'a Task) -> Self {
        let user_space = task.user_space().unwrap();
        debug_assert!(Arc::ptr_eq(
            task.user_space().unwrap(),
            Task::current().unwrap().user_space().unwrap()
        ));
        Self(user_space.vm_space())
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if the `vaddr` and `len` do not represent a user space memory range.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        Ok(self.0.reader(vaddr, len)?)
    }

    /// Creates a writer to write data into the user space.
    ///
    /// Returns `Err` if the `vaddr` and `len` do not represent a user space memory range.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        Ok(self.0.writer(vaddr, len)?)
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

    /// Reads a C string from the user space of the current process.
    /// The length of the string should not exceed `max_len`,
    /// including the final `\0` byte.
    pub fn read_cstring(&self, vaddr: Vaddr, max_len: usize) -> Result<CString> {
        if max_len > 0 {
            check_vaddr(vaddr)?;
        }

        let mut user_reader = self.reader(vaddr, max_len)?;
        user_reader.read_cstring()
    }
}

/// A trait providing the ability to read a C string from the user space.
///
/// The user space should be of the current process. The implemented method
/// should read the bytes iteratively in the reader ([`VmReader`]) until
/// encountering the end of the reader or reading a `\0` (which is also
/// included in the final C String).
pub trait ReadCString {
    fn read_cstring(&mut self) -> Result<CString>;
}

impl ReadCString for VmReader<'_, Fallible> {
    /// Reads a C string from the user space.
    ///
    /// This implementation is inspired by
    /// the `do_strncpy_from_user` function in Linux kernel.
    /// The original Linux implementation can be found at:
    /// <https://elixir.bootlin.com/linux/v6.0.9/source/lib/strncpy_from_user.c#L28>
    fn read_cstring(&mut self) -> Result<CString> {
        let max_len = self.remain();
        let mut buffer: Vec<u8> = Vec::with_capacity(max_len);

        macro_rules! read_one_byte_at_a_time_while {
            ($cond:expr) => {
                while $cond {
                    let byte = self.read_val::<u8>()?;
                    buffer.push(byte);
                    if byte == 0 {
                        return Ok(CString::from_vec_with_nul(buffer)
                            .expect("We provided 0 but no 0 is found"));
                    }
                }
            };
        }

        // Handle the first few bytes to make `cur_addr` aligned with `size_of::<usize>`
        read_one_byte_at_a_time_while!(
            !is_addr_aligned(self.cursor() as usize) && buffer.len() < max_len
        );

        // Handle the rest of the bytes in bulk
        while (buffer.len() + mem::size_of::<usize>()) <= max_len {
            let Ok(word) = self.read_val::<usize>() else {
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
        }

        // Handle the last few bytes that are not enough for a word
        read_one_byte_at_a_time_while!(buffer.len() < max_len);

        // Maximum length exceeded before finding the null terminator
        return_errno_with_message!(Errno::EFAULT, "Fails to read CString from user");
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
            "Bad user space pointer specified",
        ))
    } else {
        Ok(())
    }
}

/// Checks if the given address is aligned.
const fn is_addr_aligned(addr: usize) -> bool {
    (addr & (mem::size_of::<usize>() - 1)) == 0
}
