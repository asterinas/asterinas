// SPDX-License-Identifier: MPL-2.0

//! Extensions for CPU-local types that allows single-instruction operations.
//!
//! For some per-CPU objects, fetching or modifying the values of them can be
//! done in a single instruction. Then we would avoid turning off interrupts
//! when accessing them, which incurs non-trivial overhead.
//!
//! These traits are the architecture-specific interface for single-instruction
//! operations. The architecture-specific module can implement these traits for
//! common integer types. For architectures that don't support such single-
//! instruction operations, we emulate a single-instruction implementation by
//! disabling interruptions and preemptions.
//!
//! Currently we implement some of the [`core::ops`] operations. Bitwise shift
//! implementations are missing. Also for less-fundamental types such as
//! enumerations or boolean types, the caller can cast it themselves to the
//! integer types, for which the operations are implemented.
//!
//! # Safety
//!
//! All operations in the provided traits are unsafe, and the caller should
//! ensure that the offset is a valid pointer to a static [`CpuLocalCell`]
//! object. The offset of the object is relative to the base address of the
//! CPU-local storage. These operations are not atomic. Accessing the same
//! address from multiple CPUs produces undefined behavior.
//!
//! [`CpuLocalCell`]: crate::cpu::local::CpuLocalCell

/// An interface for architecture-specific single-instruction add operation.
pub trait SingleInstructionAddAssign<Rhs = Self> {
    /// Adds a value to the per-CPU object.
    ///
    /// This operation wraps on overflow.
    ///
    /// # Safety
    ///
    ///
    unsafe fn add_assign(offset: *mut Self, rhs: Rhs);
}

impl<T: num_traits::WrappingAdd + Copy> SingleInstructionAddAssign<T> for T {
    default unsafe fn add_assign(offset: *mut Self, rhs: T) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset as usize) as *mut Self;
        addr.write(addr.read().wrapping_add(&rhs));
    }
}

/// An interface for architecture-specific single-instruction subtract operation.
pub trait SingleInstructionSubAssign<Rhs = Self> {
    /// Subtracts a value to the per-CPU object.
    ///
    /// This operation wraps on overflow.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    unsafe fn sub_assign(offset: *mut Self, rhs: Rhs);
}

impl<T: num_traits::WrappingSub + Copy> SingleInstructionSubAssign<T> for T {
    default unsafe fn sub_assign(offset: *mut Self, rhs: T) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset as usize) as *mut Self;
        addr.write(addr.read().wrapping_sub(&rhs));
    }
}

/// An interface for architecture-specific single-instruction bitwise OR.
pub trait SingleInstructionBitOrAssign<Rhs = Self> {
    /// Bitwise ORs a value to the per-CPU object.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    unsafe fn bitor_assign(offset: *mut Self, rhs: Rhs);
}

impl<T: core::ops::BitOr<Output = T> + Copy> SingleInstructionBitOrAssign<T> for T {
    default unsafe fn bitor_assign(offset: *mut Self, rhs: T) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset as usize) as *mut Self;
        addr.write(addr.read() | rhs);
    }
}

/// An interface for architecture-specific single-instruction bitwise AND.
pub trait SingleInstructionBitAndAssign<Rhs = Self> {
    /// Bitwise ANDs a value to the per-CPU object.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    unsafe fn bitand_assign(offset: *mut Self, rhs: Rhs);
}

impl<T: core::ops::BitAnd<Output = T> + Copy> SingleInstructionBitAndAssign<T> for T {
    default unsafe fn bitand_assign(offset: *mut Self, rhs: T) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset as usize) as *mut Self;
        addr.write(addr.read() & rhs);
    }
}

/// An interface for architecture-specific single-instruction bitwise XOR.
pub trait SingleInstructionBitXorAssign<Rhs = Self> {
    /// Bitwise XORs a value to the per-CPU object.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    #[allow(unused)]
    unsafe fn bitxor_assign(offset: *mut Self, rhs: Rhs);
}

impl<T: core::ops::BitXor<Output = T> + Copy> SingleInstructionBitXorAssign<T> for T {
    default unsafe fn bitxor_assign(offset: *mut Self, rhs: T) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset as usize) as *mut Self;
        addr.write(addr.read() ^ rhs);
    }
}

/// An interface for architecture-specific single-instruction get operation.
pub trait SingleInstructionLoad {
    /// Gets the value of the per-CPU object.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    unsafe fn load(offset: *const Self) -> Self;
}

impl<T: Copy> SingleInstructionLoad for T {
    default unsafe fn load(offset: *const Self) -> Self {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let ptr = (base + offset as usize) as *const Self;
        ptr.read()
    }
}

/// An interface for architecture-specific single-instruction set operation.
pub trait SingleInstructionStore {
    /// Writes a value to the per-CPU object.
    ///
    /// # Safety
    ///
    /// Please refer to the module-level documentation of [`self`].
    unsafe fn store(offset: *mut Self, val: Self);
}

impl<T: Copy> SingleInstructionStore for T {
    default unsafe fn store(offset: *mut Self, val: Self) {
        let _guard = crate::trap::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let ptr = (base + offset as usize) as *mut Self;
        ptr.write(val);
    }
}
