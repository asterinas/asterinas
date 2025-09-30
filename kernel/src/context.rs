// SPDX-License-Identifier: MPL-2.0

//! The context that can be accessed from the current task, thread or process.

use core::cell::Ref;

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
    vm::vmar::{Vmar, ROOT_VMAR_LOWEST_ADDR},
};

/// The context that can be accessed from the current POSIX thread.
#[derive(Clone)]
pub struct Context<'a> {
    pub process: Arc<Process>,
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

    /// Returns whether the VMAR is shared with other processes or threads.
    pub fn is_vmar_shared(&self) -> bool {
        // If the VMAR is not shared, its reference count should be exactly 2:
        // one reference is held by `ThreadLocal` and the other by `ProcessVm` in `Process`.
        self.root_vmar().reference_count() != 2
    }

    /// Creates a reader to read data from the user space of the current task.
    ///
    /// Returns `Err` if `vaddr` and `len` do not represent a user space memory range.
    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        // Do NOT attempt to call `check_vaddr_lowerbound` here.
        //
        // Linux has a **delayed buffer validation** behavior:
        // The Linux kernel assumes that a given user-space pointer is valid until it attempts to access it.
        // For example, the following invocation of the `read` system call with a `NULL` pointer as the buffer
        //
        // ```c
        // read(fd, NULL, 1);
        // ```
        //
        // will return 0 (rather than an error) if the file referred to by `fd` has zero length.
        //
        // Asterinas's system call entry points follow a pattern of converting user-space pointers to
        // a reader/writer first and using the reader/writer later.
        // So adding any pointer check here would break Asterinas's delayed buffer validation behavior.
        Ok(self.root_vmar().vm_space().reader(vaddr, len)?)
    }

    /// Creates a writer to write data into the user space of the current task.
    ///
    /// Returns `Err` if `vaddr` and `len` do not represent a user space memory range.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        // Do NOT attempt to call `check_vaddr_lowerbound` here.
        // See the comments in the `reader` method.
        Ok(self.root_vmar().vm_space().writer(vaddr, len)?)
    }

    /// Creates a reader/writer pair to read data from or write data into the user space
    /// of the current task.
    ///
    /// Returns `Err` if `vaddr` and `len` do not represent a user space memory range.
    ///
    /// This method is semantically equivalent to calling [`Self::reader`] and [`Self::writer`]
    /// separately, but it avoids double checking the validity of the memory region.
    pub fn reader_writer(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> Result<(VmReader<'_, Fallible>, VmWriter<'_, Fallible>)> {
        // Do NOT attempt to call `check_vaddr_lowerbound` here.
        // See the comments in the `reader` method.
        Ok(self.root_vmar().vm_space().reader_writer(vaddr, len)?)
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
            check_vaddr_lowerbound(src)?;
        }

        let mut user_reader = self.reader(src, copy_len)?;
        user_reader.read_fallible(dest).map_err(|err| err.0)?;
        Ok(())
    }

    /// Reads a POD value from the user space of the current process.
    pub fn read_val<T: Pod>(&self, src: Vaddr) -> Result<T> {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(src)?;
        }

        let mut user_reader = self.reader(src, size_of::<T>())?;
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
            check_vaddr_lowerbound(dest)?;
        }

        let mut user_writer = self.writer(dest, copy_len)?;
        user_writer.write_fallible(src).map_err(|err| err.0)?;
        Ok(())
    }

    /// Writes a POD value to the user space of the current process.
    pub fn write_val<T: Pod>(&self, dest: Vaddr, val: &T) -> Result<()> {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(dest)?;
        }

        let mut user_writer = self.writer(dest, size_of::<T>())?;
        Ok(user_writer.write_val(val)?)
    }

    /// Atomically loads a `PodAtomic` value with [`Ordering::Relaxed`] semantics.
    ///
    /// # Panics
    ///
    /// This method will panic if `vaddr` is not aligned on an `align_of::<T>()`-byte boundary.
    ///
    /// [`Ordering::Relaxed`]: core::sync::atomic::Ordering::Relaxed
    pub fn atomic_load<T: PodAtomic>(&self, vaddr: Vaddr) -> Result<T> {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        let user_reader = self.reader(vaddr, size_of::<T>())?;
        Ok(user_reader.atomic_load()?)
    }

    /// Atomically updates a `PodAtomic` value with [`Ordering::Relaxed`] semantics.
    ///
    /// This method internally fetches the old value via [`atomic_load`], applies `op` to compute a
    /// new value, and updates the value via [`atomic_compare_exchange`]. If the value changes
    /// concurrently, this method will retry so the operation may be performed multiple times.
    ///
    /// If the update is completely successful, returns `Ok` with the old value (i.e., the value
    /// _before_ applying `op`). Otherwise, it returns `Err`.
    ///
    /// # Panics
    ///
    /// This method will panic if `vaddr` is not aligned on an `align_of::<T>()`-byte boundary.
    ///
    /// [`Ordering::Relaxed`]: core::sync::atomic::Ordering::Relaxed
    /// [`atomic_load`]: VmReader::atomic_load
    /// [`atomic_compare_exchange`]: VmWriter::atomic_compare_exchange
    pub fn atomic_fetch_update<T>(&self, vaddr: Vaddr, op: impl Fn(T) -> T) -> Result<T>
    where
        T: PodAtomic + Eq,
    {
        if size_of::<T>() > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        let (reader, writer) = self.reader_writer(vaddr, size_of::<T>())?;

        let mut old_val = reader.atomic_load()?;
        loop {
            match writer.atomic_compare_exchange(&reader, old_val, op(old_val))? {
                (_, true) => return Ok(old_val),
                (cur_val, false) => old_val = cur_val,
            }
        }
    }

    /// Reads a C string from the user space of the current process.
    ///
    /// The length of the string should not exceed `max_len`, including the final nul byte.
    /// Otherwise, this method will fail with [`Errno::ENAMETOOLONG`].
    ///
    /// This method is commonly used to read a file name or path. In that case, when the nul byte
    /// cannot be found within `max_len` bytes, the correct error code is [`Errno::ENAMETOOLONG`].
    /// However, in other cases, the caller may want to fix the error code manually.
    pub fn read_cstring(&self, vaddr: Vaddr, max_len: usize) -> Result<CString> {
        if max_len > 0 {
            check_vaddr_lowerbound(vaddr)?;
        }

        // Adjust `max_len` to ensure `vaddr + max_len` does not exceed `MAX_USERSPACE_VADDR`.
        // If `vaddr` is outside user address space, `userspace_max_len` will be set to zero and
        // further call to `self.reader` will return `EFAULT`.
        let userspace_max_len = MAX_USERSPACE_VADDR.saturating_sub(vaddr).min(max_len);

        let mut user_reader = self.reader(vaddr, userspace_max_len)?;
        user_reader.read_cstring_until_nul(userspace_max_len)?
            .ok_or_else(|| if userspace_max_len == max_len {
                // There may be more bytes in the userspace, but the length limit has been reached.
                Error::with_message(
                    Errno::ENAMETOOLONG,
                    "the C string does not end before reaching the maximum length"
                )
            } else {
                // There cannot be any bytes in the userspace, but the C string still does not end.
                // This is the Linux behavior in its `do_strncpy_from_user` implementation.
                Error::with_message(
                    Errno::EFAULT,
                    "the C string does not end before reaching the maximum userspace virtual address"
                )
            })
    }
}

/// Checks if the user space pointer is below the lowest userspace address.
///
/// If a pointer is below the lowest userspace address, it is likely to be a
/// NULL pointer. Reading from or writing to a NULL pointer should trigger a
/// segmentation fault.
///
/// If it is not checked here, a kernel page fault will happen and we would
/// deny the access in the page fault handler anyway. It may save a page fault
/// in some occasions. More importantly, double page faults may not be handled
/// quite well on some platforms.
fn check_vaddr_lowerbound(va: Vaddr) -> Result<()> {
    if va < ROOT_VMAR_LOWEST_ADDR {
        return_errno_with_message!(Errno::EFAULT, "the userspace address is too small");
    }
    Ok(())
}
