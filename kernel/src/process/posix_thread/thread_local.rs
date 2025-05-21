// SPDX-License-Identifier: MPL-2.0

use core::cell::{Cell, Ref, RefCell, RefMut};

use aster_rights::Full;
use ostd::{mm::Vaddr, sync::RwArc, task::CurrentTask};

use super::RobustListHead;
use crate::{fs::file_table::FileTable, process::signal::SigStack, vm::vmar::Vmar};

/// Local data for a POSIX thread.
pub struct ThreadLocal {
    // TID pointers.
    // https://man7.org/linux/man-pages/man2/set_tid_address.2.html
    set_child_tid: Cell<Vaddr>,
    clear_child_tid: Cell<Vaddr>,

    // Virtual memory address regions.
    root_vmar: RefCell<Option<Vmar<Full>>>,

    // Robust futexes.
    // https://man7.org/linux/man-pages/man2/get_robust_list.2.html
    robust_list: RefCell<Option<RobustListHead>>,

    // Files.
    file_table: RefCell<Option<RwArc<FileTable>>>,

    // Signal.
    /// `ucontext` address for the signal handler.
    // FIXME: This field may be removed. For glibc applications with RESTORER flag set, the
    // `sig_context` is always equals with RSP.
    sig_context: Cell<Option<Vaddr>>,
    /// Stack address, size, and flags for the signal handler.
    sig_stack: RefCell<Option<SigStack>>,
}

impl ThreadLocal {
    pub(super) fn new(
        set_child_tid: Vaddr,
        clear_child_tid: Vaddr,
        root_vmar: Vmar<Full>,
        file_table: RwArc<FileTable>,
    ) -> Self {
        Self {
            set_child_tid: Cell::new(set_child_tid),
            clear_child_tid: Cell::new(clear_child_tid),
            root_vmar: RefCell::new(Some(root_vmar)),
            robust_list: RefCell::new(None),
            file_table: RefCell::new(Some(file_table)),
            sig_context: Cell::new(None),
            sig_stack: RefCell::new(None),
        }
    }

    pub fn set_child_tid(&self) -> &Cell<Vaddr> {
        &self.set_child_tid
    }

    pub fn clear_child_tid(&self) -> &Cell<Vaddr> {
        &self.clear_child_tid
    }

    pub fn root_vmar(&self) -> &RefCell<Option<Vmar<Full>>> {
        &self.root_vmar
    }

    pub fn robust_list(&self) -> &RefCell<Option<RobustListHead>> {
        &self.robust_list
    }

    pub fn borrow_file_table(&self) -> FileTableRef {
        FileTableRef(self.file_table.borrow())
    }

    pub fn borrow_file_table_mut(&self) -> FileTableRefMut {
        FileTableRefMut(self.file_table.borrow_mut())
    }

    pub fn sig_context(&self) -> &Cell<Option<Vaddr>> {
        &self.sig_context
    }

    pub fn sig_stack(&self) -> &RefCell<Option<SigStack>> {
        &self.sig_stack
    }
}

/// An immutable, shared reference to the file table in [`ThreadLocal`].
pub struct FileTableRef<'a>(Ref<'a, Option<RwArc<FileTable>>>);

impl FileTableRef<'_> {
    /// Unwraps and returns a reference to the file table.
    ///
    /// # Panics
    ///
    /// This method will panic if the thread has exited and the file table has been dropped.
    pub fn unwrap(&self) -> &RwArc<FileTable> {
        self.0.as_ref().unwrap()
    }
}

/// A mutable, exclusive reference to the file table in [`ThreadLocal`].
pub struct FileTableRefMut<'a>(RefMut<'a, Option<RwArc<FileTable>>>);

impl FileTableRefMut<'_> {
    /// Unwraps and returns a reference to the file table.
    ///
    /// # Panics
    ///
    /// This method will panic if the thread has exited and the file table has been dropped.
    pub fn unwrap(&mut self) -> &mut RwArc<FileTable> {
        self.0.as_mut().unwrap()
    }

    /// Removes the file table and drops it.
    pub(super) fn remove(&mut self) {
        *self.0 = None;
    }
}

/// A trait to provide the `as_thread_local` method for tasks.
pub trait AsThreadLocal {
    /// Returns the associated [`ThreadLocal`].
    fn as_thread_local(&self) -> Option<&ThreadLocal>;
}

impl AsThreadLocal for CurrentTask {
    fn as_thread_local(&self) -> Option<&ThreadLocal> {
        self.local_data().downcast_ref()
    }
}
