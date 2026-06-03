// SPDX-License-Identifier: MPL-2.0

use core::cell::{Cell, Ref, RefCell, RefMut};

#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::{FsBase, GsBase};
use ostd::{
    arch::cpu::context::FpuContext, irq::DisabledLocalIrqGuard, sync::RwArc, task::CurrentTask,
};

use super::{RobustListHead, cpu_sync::CpuSync};
use crate::{
    fs::{file::file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        NsProxy, UserNamespace,
        signal::{SigStack, sig_mask::SigMask},
    },
    vm::vmar::VmarHandle,
};

/// Local data for a POSIX thread.
pub struct ThreadLocal {
    // TID pointers.
    // https://man7.org/linux/man-pages/man2/set_tid_address.2.html
    set_child_tid: Cell<Vaddr>,
    clear_child_tid: Cell<Vaddr>,

    // Virtual memory address regions.
    vmar: RefCell<Option<VmarHandle>>,
    page_fault_disabled: Cell<bool>,

    // Robust futexes.
    // https://man7.org/linux/man-pages/man2/get_robust_list.2.html
    robust_list: RefCell<Option<RobustListHead>>,

    // Files.
    /// File table.
    file_table: RefCell<Option<RwArc<FileTable>>>,
    /// File system.
    fs: RefCell<Arc<ThreadFsInfo>>,

    // Supplementary userspace CPU context.
    supp_user_context: SuppUserContext,

    // Signal.
    /// Stack address, size, and flags for the signal handler.
    sig_stack: RefCell<SigStack>,
    /// Saved signal mask. It will be restored either after the signal handler, or upon
    /// return from the system call if there is no signal handler to run.
    sig_mask_saved: Cell<Option<SigMask>>,
    /// Original syscall-return register value captured
    /// at the most recent kernel entry, or `None` for non-syscall entries.
    orig_syscall_ret: Cell<Option<usize>>,

    // Namespaces.
    user_ns: RefCell<Arc<UserNamespace>>,
    ns_proxy: RefCell<Option<Arc<NsProxy>>>,
}

impl ThreadLocal {
    #[expect(clippy::too_many_arguments)]
    pub(super) fn new(
        set_child_tid: Vaddr,
        clear_child_tid: Vaddr,
        vmar: VmarHandle,
        file_table: RwArc<FileTable>,
        fs: Arc<ThreadFsInfo>,
        supp_user_context: SuppUserContext,
        user_ns: Arc<UserNamespace>,
        ns_proxy: Arc<NsProxy>,
    ) -> Self {
        Self {
            set_child_tid: Cell::new(set_child_tid),
            clear_child_tid: Cell::new(clear_child_tid),
            vmar: RefCell::new(Some(vmar)),
            page_fault_disabled: Cell::new(false),
            robust_list: RefCell::new(None),
            file_table: RefCell::new(Some(file_table)),
            fs: RefCell::new(fs),
            supp_user_context,
            sig_stack: RefCell::new(SigStack::default()),
            sig_mask_saved: Cell::new(None),
            orig_syscall_ret: Cell::new(None),
            user_ns: RefCell::new(user_ns),
            ns_proxy: RefCell::new(Some(ns_proxy)),
        }
    }

    pub fn set_child_tid(&self) -> &Cell<Vaddr> {
        &self.set_child_tid
    }

    pub fn clear_child_tid(&self) -> &Cell<Vaddr> {
        &self.clear_child_tid
    }

    pub fn vmar(&self) -> &RefCell<Option<VmarHandle>> {
        &self.vmar
    }

    /// Executes the closure with the page fault handler disabled.
    ///
    /// When page faults occur, the handler may attempt to load the page from the disk, which can break
    /// the atomic mode. By using this method, the page fault handler will fail immediately, so
    /// fallible memory operation will return [`Errno::EFAULT`] once it triggers a page fault.
    ///
    /// Usually, we should _not_ try to access the userspace memory while being in the atomic mode
    /// (e.g., when holding a spin lock). If we must do so, this method is a last resort that disables
    /// the handler instead.
    ///
    /// Note that the closure runs with different semantics of the fallible memory operation.
    /// Therefore, if it fails with a [`Errno::EFAULT`], this method will return [`None`] and it is
    /// the caller's responsibility to exit the atomic mode, handle the page fault, and retry. Do
    /// _not_ use this method without adding code that explicitly handles the page fault!
    pub fn with_page_fault_disabled<F, T>(&self, func: F) -> Option<Result<T>>
    where
        F: FnOnce() -> Result<T>,
    {
        let is_disabled = self.is_page_fault_disabled();
        self.page_fault_disabled.set(true);

        let result = func();

        self.page_fault_disabled.set(is_disabled);

        if result
            .as_ref()
            .is_err_and(|err| err.error() == Errno::EFAULT)
        {
            None
        } else {
            Some(result)
        }
    }

    pub fn is_page_fault_disabled(&self) -> bool {
        self.page_fault_disabled.get()
    }

    pub fn robust_list(&self) -> &RefCell<Option<RobustListHead>> {
        &self.robust_list
    }

    pub fn borrow_file_table(&self) -> FileTableRef<'_> {
        ThreadLocalOptionRef(self.file_table.borrow())
    }

    pub fn borrow_file_table_mut(&self) -> FileTableRefMut<'_> {
        ThreadLocalOptionRefMut(self.file_table.borrow_mut())
    }

    pub fn borrow_fs(&self) -> Ref<'_, Arc<ThreadFsInfo>> {
        self.fs.borrow()
    }

    pub(in crate::process) fn borrow_fs_mut(&self) -> RefMut<'_, Arc<ThreadFsInfo>> {
        self.fs.borrow_mut()
    }

    pub fn is_fs_shared(&self) -> bool {
        // If the filesystem information is not shared, its reference count should be exactly 2:
        // one reference is held by `ThreadLocal` and the other by `PosixThread`.
        Arc::strong_count(&self.fs.borrow()) > 2
    }

    pub fn supp_user_context(&self) -> &SuppUserContext {
        &self.supp_user_context
    }

    pub fn sig_stack(&self) -> &RefCell<SigStack> {
        &self.sig_stack
    }

    pub(in crate::process) fn sig_mask_saved(&self) -> &Cell<Option<SigMask>> {
        &self.sig_mask_saved
    }

    /// Returns the original syscall-return register value
    /// for the most recent kernel entry.
    pub(in crate::process) fn orig_syscall_ret(&self) -> Option<usize> {
        self.orig_syscall_ret.get()
    }

    /// Sets the original syscall-return register value
    /// for the most recent kernel entry.
    pub fn set_orig_syscall_ret(&self, value: Option<usize>) {
        self.orig_syscall_ret.set(value);
    }

    pub fn borrow_user_ns(&self) -> Ref<'_, Arc<UserNamespace>> {
        self.user_ns.borrow()
    }

    pub fn borrow_ns_proxy(&self) -> NsProxyRef<'_> {
        ThreadLocalOptionRef(self.ns_proxy.borrow())
    }

    pub(in crate::process) fn borrow_ns_proxy_mut(&self) -> NsProxyRefMut<'_> {
        ThreadLocalOptionRefMut(self.ns_proxy.borrow_mut())
    }
}

/// Supplementary userspace CPU context.
///
/// # `UserContext` vs `SuppUserContext`.
///
/// The entire userspace CPU state is split into two structs:
/// `UserContext` and `SuppUserContext`.
/// `UserContext` is a set of essential CPU registers,
/// including the general-purpose ones,
/// that are used by the kernel as well.
/// Thus, to avoid conflicts between the userspace and kernel space use,
/// `UserContext` has to be saved/restored _eagerly_ upon every exit/enter from/to the userspace.
/// For best performance,
/// the number of registers in `UserContext` is kept as small as possible.
///
/// In contrast,
/// `SuppUserContext` is a set of supplementary CPU registers,
/// such as FPU registers,
/// that are modifiable by the userspace
/// but might or might not be used in the kernel space.
/// As such,
/// the saving and restoring of `SuppUserContext` can be delayed to
/// the point of context switching,
/// when saving and restoring are unavoidable.
///
/// In conclusion,
/// dividing userspace CPU states into `UserContext` and `SuppUserContext`
/// allows for better performance.
pub struct SuppUserContext {
    fpu: CpuSync<FpuContext>,
    #[cfg(target_arch = "x86_64")]
    fs_base: CpuSync<FsBase>,
    #[cfg(target_arch = "x86_64")]
    gs_base: CpuSync<GsBase>,
}

impl SuppUserContext {
    pub fn new() -> Self {
        Self {
            fpu: CpuSync::new(FpuContext::new()),
            #[cfg(target_arch = "x86_64")]
            fs_base: CpuSync::new(FsBase::default()),
            #[cfg(target_arch = "x86_64")]
            gs_base: CpuSync::new(GsBase::default()),
        }
    }

    pub fn with_fpu_context(mut self, ctx: FpuContext) -> Self {
        self.fpu = CpuSync::new(ctx);
        self
    }

    #[cfg(target_arch = "x86_64")]
    pub fn with_fs_base(mut self, fs_base: FsBase) -> Self {
        self.fs_base = CpuSync::new(fs_base);
        self
    }

    #[cfg(target_arch = "x86_64")]
    pub fn with_gs_base(mut self, gs_base: GsBase) -> Self {
        self.gs_base = CpuSync::new(gs_base);
        self
    }

    pub fn fpu(&self) -> &CpuSync<FpuContext> {
        &self.fpu
    }

    #[cfg(target_arch = "x86_64")]
    pub fn fs_base(&self) -> &CpuSync<FsBase> {
        &self.fs_base
    }

    #[cfg(target_arch = "x86_64")]
    pub fn gs_base(&self) -> &CpuSync<GsBase> {
        &self.gs_base
    }

    pub fn before_schedule(&self, guard: &DisabledLocalIrqGuard) {
        self.fpu.before_schedule(guard);
        #[cfg(target_arch = "x86_64")]
        {
            self.fs_base.before_schedule(guard);
            self.gs_base.before_schedule(guard);
        }
    }

    pub fn before_user_exec(&self, guard: &DisabledLocalIrqGuard) {
        self.fpu.before_user_exec(guard);
        #[cfg(target_arch = "x86_64")]
        {
            self.fs_base.before_user_exec(guard);
            self.gs_base.before_user_exec(guard);
        }
    }
}

/// An immutable, shared reference to the file table in [`ThreadLocal`].
pub type FileTableRef<'a> = ThreadLocalOptionRef<'a, RwArc<FileTable>>;

/// An immutable, shared reference to the `NsProxy` in [`ThreadLocal`].
pub type NsProxyRef<'a> = ThreadLocalOptionRef<'a, Arc<NsProxy>>;

/// An immutable, shared reference to thread-local data contained within a `RefCell<Option<..>>`.
pub struct ThreadLocalOptionRef<'a, T>(Ref<'a, Option<T>>);

impl<T> ThreadLocalOptionRef<'_, T> {
    /// Unwraps and returns a reference to the data.
    ///
    /// # Panics
    ///
    /// This method will panic if the thread has exited and the data has been dropped.
    pub fn unwrap(&self) -> &T {
        self.0.as_ref().unwrap()
    }
}

/// A mutable, exclusive reference to the file table in [`ThreadLocal`].
pub type FileTableRefMut<'a> = ThreadLocalOptionRefMut<'a, RwArc<FileTable>>;

/// A mutable, exclusive reference to the `NsProxy` in [`ThreadLocal`].
pub(in crate::process) type NsProxyRefMut<'a> = ThreadLocalOptionRefMut<'a, Arc<NsProxy>>;

/// A mutable, exclusive reference to thread-local data contained within a `RefCell<Option<..>>`.
pub struct ThreadLocalOptionRefMut<'a, T>(RefMut<'a, Option<T>>);

impl<T> ThreadLocalOptionRefMut<'_, T> {
    /// Unwraps and returns a reference to the data.
    ///
    /// # Panics
    ///
    /// This method will panic if the thread has exited and the data has been dropped.
    pub fn unwrap(&mut self) -> &mut T {
        self.0.as_mut().unwrap()
    }

    /// Removes the data and returns it.
    pub(super) fn remove(&mut self) -> Option<T> {
        self.0.take()
    }

    /// Replaces the data with a new one, returning the old one.
    pub(in crate::process) fn replace(&mut self, new: Option<T>) -> Option<T> {
        core::mem::replace(&mut *self.0, new)
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
