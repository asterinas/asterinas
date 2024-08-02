// SPDX-License-Identifier: MPL-2.0

//! Extensions for CPU-local types that allows single-instruction operations.

use super::CpuLocal;

/// Architecture-specific implementations of single-instruction operations.
///
/// For some per-CPU objects, fetching or modifying the values of them can be
/// done in a single instruction. Then we would avoid turning off interrupts
/// when accessing them, which incurs non-trivial overhead.
///
/// This trait is the architecture-specific interface for single-instruction
/// operations. The architecture-specific module can implement this trait for
/// common integer types.
///
/// Currently we don't plan to implement most of the [`core::ops`] operations
/// for the sake of simplicity. The status-quos are sufficient enough.
pub trait SingleInstructionOps {
    /// The right hand side type for the operations.
    type Rhs;

    /// Adds a value to the per-CPU object.
    ///
    /// This operation wraps on overflow.
    ///
    /// The offset of the object is relative to the base address of the CPU-
    /// local storage.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the offset is valid to be written to.
    /// This operation is not atomic. Accessing the same address from multiple
    /// threads produces undefined behavior.
    unsafe fn cpu_local_add_assign(offset: *mut Self, rhs: Self::Rhs);

    /// Subtracts a value to the per-CPU object.
    ///
    /// This operation wraps on overflow.
    ///
    /// The offset of the object is relative to the base address of the CPU-
    /// local storage.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the offset is valid to be written to.
    /// This operation is not atomic. Accessing the same address from multiple
    /// threads produces undefined behavior.
    unsafe fn cpu_local_sub_assign(offset: *mut Self, rhs: Self::Rhs);

    /// Gets the value of the per-CPU object.
    ///
    /// The offset of the object is relative to the base address of the CPU-
    /// local storage.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the offset is valid to be read from.
    /// This operation is not atomic. Accessing the same address from multiple
    /// threads produces undefined behavior.
    unsafe fn cpu_local_read(offset: *const Self) -> Self;

    /// Writes a value to the per-CPU object.
    ///
    /// The offset of the object is relative to the base address of the CPU-
    /// local storage.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the offset is valid to be written to.
    /// This operation is not atomic. Accessing the same address from multiple
    /// threads produces undefined behavior.
    unsafe fn cpu_local_write(offset: *mut Self, val: Self);
}

impl<T: SingleInstructionOps> CpuLocal<T> {
    /// Adds a value to the per-CPU object in a single instruction.
    ///
    /// # Safety
    ///
    /// Since the operation does not forbid shared references, the caller
    /// should ensure that the object is not accessed by multiple tasks.
    /// The safety requirement for the object is the same as [`UnsafeCell`].
    ///
    /// [`UnsafeCell`]: core::cell::UnsafeCell
    pub unsafe fn add_assign(&self, rhs: T::Rhs) {
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid.
        unsafe {
            T::cpu_local_add_assign(self as *const _ as usize as *mut T, rhs);
        }
    }

    /// Subtracts a value to the per-CPU object in a single instruction.
    ///
    /// # Safety
    ///
    /// Since the operation does not forbid shared references, the caller
    /// should ensure that the object is not accessed by multiple tasks.
    /// The safety requirement for the object is the same as [`UnsafeCell`].
    ///
    /// [`UnsafeCell`]: core::cell::UnsafeCell
    pub unsafe fn sub_assign(&self, rhs: T::Rhs) {
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid.
        unsafe {
            T::cpu_local_sub_assign(self as *const _ as usize as *mut T, rhs);
        }
    }

    /// Gets the value of the per-CPU object in a single instruction.
    ///
    /// # Safety
    ///
    /// Since the operation does not forbid shared references, the caller
    /// should ensure that the object is not accessed by multiple tasks.
    /// The safety requirement for the object is the same as [`UnsafeCell`].
    ///
    /// [`UnsafeCell`]: core::cell::UnsafeCell
    pub unsafe fn read(&self) -> T {
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid.
        unsafe { T::cpu_local_read(self as *const _ as usize as *const T) }
    }

    /// Writes a value to the per-CPU object in a single instruction.
    ///
    /// # Safety
    ///
    /// Since the operation does not forbid shared references, the caller
    /// should ensure that the object is not accessed by multiple tasks.
    /// The safety requirement for the object is the same as [`UnsafeCell`].
    ///
    /// [`UnsafeCell`]: core::cell::UnsafeCell
    pub unsafe fn write(&mut self, val: T) {
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid.
        unsafe {
            T::cpu_local_write(self as *const _ as usize as *mut T, val);
        }
    }
}
