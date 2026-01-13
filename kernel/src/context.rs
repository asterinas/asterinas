// SPDX-License-Identifier: MPL-2.0

//! The context that can be accessed from the current task, thread or process.

use core::cell::Ref;

use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{Fallible, PodAtomic, VmIo, VmReader, VmWriter},
    task::Task,
};

use crate::{
    prelude::*,
    process::{
        Process,
        posix_thread::{PosixThread, ThreadLocal},
    },
    thread::Thread,
    vm::vmar::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, Vmar},
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
    pub fn user_space(&self) -> CurrentUserSpace<'_> {
        CurrentUserSpace(self.thread_local.vmar().borrow())
    }
}

/// The user's memory space of the current task.
///
/// It provides methods to read from or write to the user space efficiently.
//
// FIXME: With `impl VmIo for &CurrentUserSpace<'_>`, the Rust compiler seems to think that
// `CurrentUserSpace` is a publicly exposed type, despite the fact that it is contained in a
// private module and is never actually exposed. Consequently, it incorrectly suppresses many dead
// code lints (for *lots of* types that are recursively reached via `CurrentUserSpace`'s APIs). As
// a workaround, we mark the type as `pub(crate)`. We can restore it to `pub` once the compiler bug
// is resolved.
pub(crate) struct CurrentUserSpace<'a>(Ref<'a, Option<Arc<Vmar>>>);

/// Gets the [`CurrentUserSpace`] from the current task.
///
/// This is slower than [`Context::user_space`]. Don't use this getter
/// If you get the access to the [`Context`].
#[macro_export]
macro_rules! current_userspace {
    () => {
        $crate::context::CurrentUserSpace::new(
            $crate::process::posix_thread::AsThreadLocal::as_thread_local(
                &ostd::task::Task::current().unwrap(),
            )
            .unwrap(),
        )
    };
}

impl<'a> CurrentUserSpace<'a> {
    /// Creates a new `CurrentUserSpace` from the current task.
    ///
    /// If you have access to a [`Context`], it is preferable to call [`Context::user_space`].
    ///
    /// Otherwise, you can use the `current_userspace` macro
    /// to obtain an instance of `CurrentUserSpace` if it will only be used once.
    pub fn new(thread_local: &'a ThreadLocal) -> Self {
        let vmar_ref = thread_local.vmar().borrow();
        Self(vmar_ref)
    }

    /// Returns the `Vmar` of the current userspace.
    ///
    /// # Panics
    ///
    /// This method will panic if the current process has cleared its `Vmar`.
    pub fn vmar(&self) -> &Vmar {
        self.0.as_ref().unwrap()
    }

    /// Returns whether the VMAR is shared with other processes or threads.
    pub fn is_vmar_shared(&self) -> bool {
        // If the VMAR is not shared, its reference count should be exactly 2:
        // one reference is held by `ThreadLocal` and the other by `ProcessVm` in `Process`.
        Arc::strong_count(self.0.as_ref().unwrap()) > 2
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
        Ok(self.vmar().vm_space().reader(vaddr, len)?)
    }

    /// Creates a writer to write data into the user space of the current task.
    ///
    /// Returns `Err` if `vaddr` and `len` do not represent a user space memory range.
    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        // Do NOT attempt to call `check_vaddr_lowerbound` here.
        // See the comments in the `reader` method.
        Ok(self.vmar().vm_space().writer(vaddr, len)?)
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
        Ok(self.vmar().vm_space().reader_writer(vaddr, len)?)
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

        // Adjust `max_len` to ensure `vaddr + max_len` does not exceed `VMAR_CAP_ADDR`.
        // If `vaddr` is outside user address space, `userspace_max_len` will be set to zero and
        // further call to `self.reader` will return `EFAULT`.
        let userspace_max_len = VMAR_CAP_ADDR.saturating_sub(vaddr).min(max_len);

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

impl VmIo for CurrentUserSpace<'_> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        let copy_len = writer.avail();

        if copy_len > 0 {
            check_vaddr_lowerbound(offset)?;
        }

        let mut user_reader = self.vmar().vm_space().reader(offset, copy_len)?;
        user_reader.read_fallible(writer).map_err(|err| err.0)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        let copy_len = reader.remain();

        if copy_len > 0 {
            check_vaddr_lowerbound(offset)?;
        }

        let mut user_writer = self.vmar().vm_space().writer(offset, copy_len)?;
        user_writer.write_fallible(reader).map_err(|err| err.0)?;
        Ok(())
    }
}

#[inherit_methods(from = "(**self)")]
impl VmIo for &CurrentUserSpace<'_> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()>;
    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()>;
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
fn check_vaddr_lowerbound(va: Vaddr) -> ostd::Result<()> {
    if va < VMAR_LOWEST_ADDR {
        return Err(ostd::Error::PageFault);
    }
    Ok(())
}
