// SPDX-License-Identifier: MPL-2.0

use core::cell::{Cell, Ref, RefCell, RefMut};

use aster_rights::Full;
use ostd::{cpu::context::FpuContext, mm::Vaddr, sync::RwArc, task::CurrentTask};

use super::RobustListHead;
use crate::{
    fs::{file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::signal::SigStack,
    vm::vmar::Vmar,
};

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
    /// File table.
    file_table: RefCell<Option<RwArc<FileTable>>>,
    /// File system.
    fs: RefCell<Arc<ThreadFsInfo>>,

    // User FPU context.
    fpu_context: RefCell<FpuContext>,
    fpu_state: Cell<FpuState>,

    // Signal.
    /// `ucontext` address for the signal handler.
    // FIXME: This field may be removed. For glibc applications with RESTORER flag set, the
    // `sig_context` is always equals with RSP.
    sig_context: Cell<Option<Vaddr>>,
    /// Stack address, size, and flags for the signal handler.
    sig_stack: RefCell<SigStack>,
}

impl ThreadLocal {
    pub(super) fn new(
        set_child_tid: Vaddr,
        clear_child_tid: Vaddr,
        root_vmar: Vmar<Full>,
        file_table: RwArc<FileTable>,
        fs: Arc<ThreadFsInfo>,
        fpu_context: FpuContext,
    ) -> Self {
        Self {
            set_child_tid: Cell::new(set_child_tid),
            clear_child_tid: Cell::new(clear_child_tid),
            root_vmar: RefCell::new(Some(root_vmar)),
            robust_list: RefCell::new(None),
            file_table: RefCell::new(Some(file_table)),
            fs: RefCell::new(fs),
            sig_context: Cell::new(None),
            sig_stack: RefCell::new(SigStack::default()),
            fpu_context: RefCell::new(fpu_context),
            fpu_state: Cell::new(FpuState::Unloaded),
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

    pub fn borrow_fs(&self) -> Ref<'_, Arc<ThreadFsInfo>> {
        self.fs.borrow()
    }

    pub fn borrow_fs_mut(&self) -> RefMut<'_, Arc<ThreadFsInfo>> {
        self.fs.borrow_mut()
    }

    pub fn sig_context(&self) -> &Cell<Option<Vaddr>> {
        &self.sig_context
    }

    pub fn sig_stack(&self) -> &RefCell<SigStack> {
        &self.sig_stack
    }

    pub fn fpu(&self) -> ThreadFpu<'_> {
        ThreadFpu(self)
    }
}

/// The current state of `ThreadFpu`.
///
/// - `Activated`: The FPU context is currently loaded onto the CPU and it must be loaded
///   while the associated task is running. If preemption occurs in between, the context switch
///   must load FPU context again.
/// - `Loaded`: The FPU context is currently loaded onto the CPU. It may or may not still
///   be loaded in CPU after a context switch.
/// - `Unloaded`: The FPU context is not currently loaded onto the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FpuState {
    Activated,
    Loaded,
    Unloaded,
}

/// The FPU information for the _current_ thread.
///
/// # Notes about kernel preemption
///
/// All the methods of `ThreadFpu` assume that preemption will not occur.
/// This means that the FPU state will not change unexpectedly
/// (e.g., changing from `Loaded` to `Unloaded`).
///
/// In the current architecture, this is always true because kernel
/// preemption was never implemented. More importantly, we cannot implement
/// kernel preemption without refactoring the `ThreadLocal` mechanism
/// because `ThreadLocal` cannot be accessed in interrupt handlers for
/// soundness reasons. But such access is necessary for the preempted
/// schedule.
///
/// Therefore, we omit the preemption guards for better performance and
/// defer preemption considerations to future work.
pub struct ThreadFpu<'a>(&'a ThreadLocal);

impl ThreadFpu<'_> {
    pub fn activate(&self) {
        match self.0.fpu_state.get() {
            FpuState::Activated => return,
            FpuState::Loaded => (),
            FpuState::Unloaded => self.0.fpu_context.borrow_mut().load(),
        }
        self.0.fpu_state.set(FpuState::Activated);
    }

    pub fn deactivate(&self) {
        if self.0.fpu_state.get() == FpuState::Activated {
            self.0.fpu_state.set(FpuState::Loaded);
        }
    }

    pub fn clone_context(&self) -> FpuContext {
        match self.0.fpu_state.get() {
            FpuState::Activated | FpuState::Loaded => {
                let mut fpu_context = self.0.fpu_context.borrow_mut();
                fpu_context.save();
                fpu_context.clone()
            }
            FpuState::Unloaded => self.0.fpu_context.borrow().clone(),
        }
    }

    pub fn set_context(&self, context: FpuContext) {
        let _ = self.0.fpu_context.replace(context);
        self.0.fpu_state.set(FpuState::Unloaded);
    }

    pub fn before_schedule(&self) {
        match self.0.fpu_state.get() {
            FpuState::Activated => {
                self.0.fpu_context.borrow_mut().save();
            }
            FpuState::Loaded => {
                self.0.fpu_context.borrow_mut().save();
                self.0.fpu_state.set(FpuState::Unloaded);
            }
            FpuState::Unloaded => (),
        }
    }

    pub fn after_schedule(&self) {
        if self.0.fpu_state.get() == FpuState::Activated {
            self.0.fpu_context.borrow_mut().load();
        }
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

    /// Replaces the file table with a new one, returning the old one.
    pub fn replace(&mut self, new_table: Option<RwArc<FileTable>>) -> Option<RwArc<FileTable>> {
        core::mem::replace(&mut *self.0, new_table)
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
