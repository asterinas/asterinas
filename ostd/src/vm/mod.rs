// SPDX-License-Identifier: MPL-2.0

//! Guest execution and physical memory management.
//!
//! OSTD provides safe interfaces for running isolated guest virtual machines.
//! A kernel supplies guest memory and CPU state, handles VM exits,
//! and implements device emulation and scheduling policy.
//!
//! # Guest memory
//!
//! A [`GuestPhysMemSpace`] maps guest physical addresses ([`Gpaddr`]s)
//! to host memory backed by untyped frames ([`UFrame`]s).
//! A kernel can load guest code and data into these frames.
//! It can query the guest memory mappings through [`Cursor`]
//! and modify them through [`CursorMut`].
//!
//! # Guest execution
//!
//! A [`GuestContext`] holds a virtual CPU's (vCPU's) register state.
//! [`GuestMode::execute`] starts or resumes guest execution in a [`GuestPhysMemSpace`]
//! from the register state in the supplied context.
//!
//! On success, execution returns a [`GuestReturnReason`]:
//!
//! - [`GuestReturnReason::VmExit`] carries exit information for the kernel to handle,
//!   such as an I/O access or a guest memory fault.
//! - [`GuestReturnReason::KernelEvent`] indicates a pending kernel event
//!   reported by [`UserModeHooks::has_kernel_event`].
//!
//! The kernel handles the return reason, updates the guest context as needed,
//! and calls [`GuestMode::execute`] again to resume the guest.
//!
//! [`UFrame`]: crate::mm::UFrame
//! [`GuestContext`]: crate::arch::vm::GuestContext
//! [`GuestMode::execute`]: crate::arch::vm::GuestMode::execute
//! [`GuestReturnReason`]: crate::arch::vm::GuestReturnReason
//! [`GuestReturnReason::VmExit`]: crate::arch::vm::GuestReturnReason::VmExit
//! [`GuestReturnReason::KernelEvent`]: crate::arch::vm::GuestReturnReason::KernelEvent
//! [`UserModeHooks::has_kernel_event`]: crate::user::UserModeHooks::has_kernel_event

mod gpm_space;
mod interrupt;
mod timer;

pub use self::{
    gpm_space::{Cursor, CursorMut, GuestPhysMemSpace, QueriedItem},
    interrupt::{DummyGuestInterruptSource, GuestInterruptSource},
    timer::{DummyGuestTimer, GuestTimer},
};

/// A guest physical address.
pub type Gpaddr = usize;
