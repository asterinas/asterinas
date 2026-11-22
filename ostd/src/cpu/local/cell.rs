// SPDX-License-Identifier: MPL-2.0

//! The implementation of CPU-local variables that have inner mutability.

use core::{cell::UnsafeCell, fmt};

use super::{__cpu_local_end, __cpu_local_start, single_instr::*};
use crate::{arch, mm::PAGE_SIZE};

/// Defines an inner-mutable CPU-local variable.
///
/// The accessors of the CPU-local variables are defined with [`CpuLocalCell`].
///
/// It should be noted that if the interrupts or preemption is enabled, two
/// operations on the same CPU-local cell variable may access different objects
/// since the task may live on different CPUs.
///
/// The alignment of the type of each variable must not exceed
/// [`PAGE_SIZE`](crate::mm::PAGE_SIZE). Otherwise, the definition does not
/// compile.
///
/// # Example
///
/// ```rust
/// use ostd::cpu_local_cell;
///
/// cpu_local_cell! {
///     static FOO: u32 = 1;
///     pub static BAR: usize = 0;
/// }
///
/// fn not_an_atomic_function() {
///     BAR.store(1);
///     // Note that the value of `BAR` here doesn't necessarily equal to `1`,
///     // since the task may be preempted and moved to another CPU.
///     // You can avoid this by disabling interrupts (and preemption, if needed).
///     println!("BAR VAL: {:?}", BAR.load());
///
///     let _irq_guard = ostd::irq::disable_local();
///     let first_foo = FOO.load();
///     // No surprises here, the two accesses must result in the same value.
///     assert_eq!(FOO.load(), first_foo);
/// }
/// ```
#[macro_export]
macro_rules! cpu_local_cell {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            // SAFETY: This is properly handled in the linker script.
            #[unsafe(link_section = ".cpu_local")]
            $(#[$attr])* $vis static $name: $crate::cpu::local::CpuLocalCell<$t> = {
                let val = $init;
                // SAFETY: The CPU local variable instantiated is statically
                // stored in the special `.cpu_local` section.
                unsafe {
                    $crate::cpu::local::CpuLocalCell::__new(val)
                }
            };
        )*
    };
}

/// Inner mutable CPU-local objects.
///
/// CPU-local cell objects are only accessible from the current CPU. When
/// accessing an underlying object using the same `CpuLocalCell` instance, the
/// actually accessed object is always on the current CPU. So in a preemptive
/// kernel task, the operated object may change if interrupts are enabled.
///
/// The inner mutability is provided by single instruction operations, and the
/// CPU-local cell objects will not ever be shared between CPUs. So it is safe
/// to modify the inner value without any locks.
///
/// You should only create the CPU-local cell object using the macro
/// [`cpu_local_cell!`].
///
/// Please exercise extreme caution when using `CpuLocalCell`. In most cases,
/// it is necessary to disable interrupts or preemption when using it to prevent
/// the operated object from being changed, which can lead to race conditions.
///
/// For the difference between [`super::CpuLocal`] and [`CpuLocalCell`], see
/// [`super`].
pub struct CpuLocalCell<T: 'static>(UnsafeCell<T>);

impl<T: 'static> CpuLocalCell<T> {
    /// Initializes a CPU-local object.
    ///
    /// Please do not call this function directly. Instead, use the
    /// `cpu_local!` macro.
    ///
    /// # Safety
    ///
    /// The caller should ensure that the object initialized by this
    /// function resides in the `.cpu_local` section. Otherwise the
    /// behavior is undefined.
    #[doc(hidden)]
    pub const unsafe fn __new(val: T) -> Self {
        // The copies of the `.cpu_local` section for the APs are only
        // page-aligned (see `copy_bsp_for_ap`), so an object keeps its
        // alignment on the APs only if the alignment is at most `PAGE_SIZE`.
        const { assert!(align_of::<T>() <= PAGE_SIZE) };
        Self(UnsafeCell::new(val))
    }

    /// Gets access to the underlying value through a raw pointer.
    ///
    /// This function calculates the virtual address of the CPU-local object
    /// based on the CPU-local base address and the offset in the BSP.
    ///
    /// This method is safe, but using the returned pointer will be unsafe.
    /// Specifically,
    /// - Preemption should be disabled from the time this method is called
    ///   to the time the pointer is used. Otherwise, the pointer may point
    ///   to the variable on another CPU, making it difficult or impossible
    ///   to determine if the data can be borrowed.
    /// - If the variable can be used in interrupt handlers, borrowing the
    ///   data should be done with interrupts disabled. Otherwise, more care
    ///   must be taken to ensure that the borrowing rules are correctly
    ///   enforced, since the interrupts may come asynchronously.
    pub fn as_mut_ptr(&'static self) -> *mut T {
        super::is_used::debug_set_true();

        let offset = {
            let bsp_va = self as *const _ as usize;
            let bsp_base = __cpu_local_start as *const () as usize;
            // The implementation should ensure that the CPU-local object resides in the `.cpu_local`.
            debug_assert!(bsp_va + size_of::<T>() <= __cpu_local_end as *const () as usize);

            bsp_va - bsp_base
        };

        let local_base = arch::cpu::local::get_base() as usize;
        let local_va = local_base + offset;

        // A sanity check about the alignment.
        debug_assert_eq!(local_va % align_of::<T>(), 0);

        local_va as *mut T
    }
}

// SAFETY: At any given time, only one task can access the inner value `T` of a
// CPU-local variable, so `T` does not have to be `Sync`. However, a value stored
// by one task can be obtained by another task, which may run on a different CPU.
// Therefore, `T` must be `Send`.
unsafe impl<T: Send + 'static> Sync for CpuLocalCell<T> {}

// Prevent valid instances of CpuLocalCell from being copied to any memory
// area outside the `.cpu_local` section.
impl<T: 'static> !Copy for CpuLocalCell<T> {}
impl<T: 'static> !Clone for CpuLocalCell<T> {}

// In general, it does not make any sense to send instances of CpuLocalCell to
// other tasks as they should live on other CPUs to make sending useful.
impl<T: 'static> !Send for CpuLocalCell<T> {}

// Accessors for the per-CPU objects whose type implements the single-
// instruction operations.

impl<T: 'static + SingleInstructionAddAssign<T>> CpuLocalCell<T> {
    /// Adds a value to the per-CPU object in a single instruction.
    ///
    /// This operation wraps on overflow/underflow.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn add_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::add_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionSubAssign<T>> CpuLocalCell<T> {
    /// Subtracts a value to the per-CPU object in a single instruction.
    ///
    /// This operation wraps on overflow/underflow.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn sub_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::sub_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitAndAssign<T>> CpuLocalCell<T> {
    /// Bitwise ANDs a value to the per-CPU object in a single instruction.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn bitand_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::bitand_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitOrAssign<T>> CpuLocalCell<T> {
    /// Bitwise ORs a value to the per-CPU object in a single instruction.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn bitor_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::bitor_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitXorAssign<T>> CpuLocalCell<T> {
    /// Bitwise XORs a value to the per-CPU object in a single instruction.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn bitxor_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::bitxor_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionLoad> CpuLocalCell<T> {
    /// Gets the value of the per-CPU object in a single instruction.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn load(&'static self) -> T {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid.
        unsafe { T::load(offset) }
    }
}

impl<T: 'static + SingleInstructionStore> CpuLocalCell<T> {
    /// Writes a value to the per-CPU object in a single instruction.
    ///
    /// Note that this memory operation will not be elided or reordered by the
    /// compiler since it is a black-box.
    pub fn store(&'static self, val: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        // SAFETY: The CPU-local object is defined in the `.cpu_local` section,
        // so the pointer to the object is valid. And the reference is never shared.
        unsafe {
            T::store(offset, val);
        }
    }
}

/// A raw pointer that can be stored in a [`CpuLocalCell`].
///
/// Raw pointers are not `Send`, so they cannot be the inner value of a
/// [`CpuLocalCell`] directly. This wrapper retains their single-instruction
/// load and store operations.
#[repr(transparent)]
pub(crate) struct CpuLocalPtr<T>(*const T);

impl<T> CpuLocalPtr<T> {
    /// Creates a new instance from a raw pointer.
    pub(crate) const fn new(ptr: *const T) -> Self {
        Self(ptr)
    }

    /// Creates a new instance with a null pointer.
    pub(crate) const fn null() -> Self {
        Self(core::ptr::null())
    }

    /// Returns the raw pointer.
    pub(crate) const fn as_ptr(self) -> *const T {
        self.0
    }
}

impl<T> Clone for CpuLocalPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CpuLocalPtr<T> {}

impl<T> fmt::Debug for CpuLocalPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CpuLocalPtr").field(&self.0).finish()
    }
}

// SAFETY: Transferring a raw pointer between tasks is harmless by itself.
// Dereferencing it requires `unsafe`, whose caller must ensure that the access
// is free of data races.
unsafe impl<T> Send for CpuLocalPtr<T> {}

impl<T> SingleInstructionLoad for CpuLocalPtr<T> {
    unsafe fn load(offset: usize) -> Self {
        // SAFETY: `CpuLocalPtr<T>` has the same layout as `*const T`, and the
        // caller upholds the safety requirements.
        Self(unsafe { <*const T as SingleInstructionLoad>::load(offset) })
    }
}

impl<T> SingleInstructionStore for CpuLocalPtr<T> {
    unsafe fn store(offset: usize, val: Self) {
        // SAFETY: `CpuLocalPtr<T>` has the same layout as `*const T`, and the
        // caller upholds the safety requirements.
        unsafe { <*const T as SingleInstructionStore>::store(offset, val.0) };
    }
}
