// SPDX-License-Identifier: MPL-2.0

use core::cell::{Cell, RefCell};

use ostd::{mm::Vaddr, task::CurrentTask};

use super::RobustListHead;
use crate::process::signal::SigStack;

/// Local data for a POSIX thread.
pub struct ThreadLocal {
    // TID pointers.
    // https://man7.org/linux/man-pages/man2/set_tid_address.2.html
    set_child_tid: Cell<Vaddr>,
    clear_child_tid: Cell<Vaddr>,

    // Robust futexes.
    // https://man7.org/linux/man-pages/man2/get_robust_list.2.html
    robust_list: RefCell<Option<RobustListHead>>,

    // Signal.
    /// `ucontext` address for the signal handler.
    // FIXME: This field may be removed. For glibc applications with RESTORER flag set, the
    // `sig_context` is always equals with RSP.
    sig_context: Cell<Option<Vaddr>>,
    /// Stack address, size, and flags for the signal handler.
    sig_stack: RefCell<Option<SigStack>>,
}

impl ThreadLocal {
    pub(super) fn new(set_child_tid: Vaddr, clear_child_tid: Vaddr) -> Self {
        Self {
            set_child_tid: Cell::new(set_child_tid),
            clear_child_tid: Cell::new(clear_child_tid),
            robust_list: RefCell::new(None),
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

    pub fn robust_list(&self) -> &RefCell<Option<RobustListHead>> {
        &self.robust_list
    }

    pub fn sig_context(&self) -> &Cell<Option<Vaddr>> {
        &self.sig_context
    }

    pub fn sig_stack(&self) -> &RefCell<Option<SigStack>> {
        &self.sig_stack
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
