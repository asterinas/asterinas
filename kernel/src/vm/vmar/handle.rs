// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::ops::Deref;

use crate::{process::ProcessVm, vm::vmar::Vmar};

/// A VMAR handle that is owned by a POSIX thread.
///
/// Each POSIX thread should hold only one VMAR handle, representing the VMAR it uses. This handle
/// should be dropped when the thread exits. Once the last handle is dropped, all mappings and page
/// tables will be cleared.
pub struct VmarHandle(Arc<Vmar>);

impl Deref for VmarHandle {
    type Target = Vmar;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for VmarHandle {
    fn drop(&mut self) {
        self.0.dec_num_handles();
    }
}

impl VmarHandle {
    /// Creates a new handle that points to a new VMAR.
    pub fn new(process_vm: ProcessVm) -> Self {
        Self(Vmar::new(process_vm))
    }

    /// Clones a new handle.
    ///
    /// This should only be used when creating a new POSIX thread that shares the same VMAR.
    pub fn clone_handle(&self) -> Self {
        self.0.inc_num_handles();
        Self(self.0.clone())
    }

    /// Clones the VMAR object with a strong reference count.
    pub fn clone_arc(&self) -> Arc<Vmar> {
        self.0.clone()
    }

    /// Clones the VMAR object with a weak reference count.
    pub fn clone_weak(&self) -> Weak<Vmar> {
        Arc::downgrade(&self.0)
    }
}
