// SPDX-License-Identifier: MPL-2.0

//! CPU local storage.
//!
//! This module provides a mechanism to define CPU-local objects.
//!
//! This is acheived by placing the CPU-local objects in a special section
//! `.cpu_local`. The bootstrap processor (BSP) uses the objects linked in this
//! section, and these objects are copied to dynamically allocated local
//! storage of each application processors (AP) during the initialization
//! process.
//!
//! Such a mechanism exploits the fact that constant values of non-[`Copy`]
//! types can be bitwise copied. For example, a [`Option<T>`] object, though
//! being not [`Copy`], have a constant constructor [`Option::None`] that
//! produces a value that can be bitwise copied to create a new instance.
//! [`alloc::sync::Arc`] however, don't have such a constructor, and thus cannot
//! be directly used as a CPU-local object. Wrapping it in a type that has a
//! constant constructor, like [`Option<T>`], can make it CPU-local.

use core::ops::Deref;

use crate::{
    arch,
    trap::{disable_local, DisabledLocalIrqGuard},
};

/// Defines a CPU-local variable.
///
/// # Example
///
/// ```rust
/// use crate::cpu_local;
/// use core::cell::RefCell;
///
/// cpu_local! {
///     static FOO: RefCell<u32> = RefCell::new(1);
///
///     #[allow(unused)]
///     pub static BAR: RefCell<f32> = RefCell::new(1.0);
/// }
///
/// println!("FOO VAL: {:?}", *FOO.borrow());
/// ```
#[macro_export]
macro_rules! cpu_local {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            #[link_section = ".cpu_local"]
            $(#[$attr])* $vis static $name: $crate::CpuLocal<$t> = {
                let val = $init;
                // SAFETY: The CPU local variable instantiated is statically
                // stored in the special `.cpu_local` section.
                unsafe {
                    $crate::CpuLocal::__new(val)
                }
            };
        )*
    };
}

/// CPU-local objects.
///
/// A CPU-local object only gives you immutable references to the underlying value.
/// To mutate the value, one can use atomic values (e.g., [`AtomicU32`]) or internally mutable
/// objects (e.g., [`RefCell`]).
///
/// [`AtomicU32`]: core::sync::atomic::AtomicU32
/// [`RefCell`]: core::cell::RefCell
pub struct CpuLocal<T>(T);

// SAFETY: At any given time, only one task can access the inner value T
// of a cpu-local variable even if `T` is not `Sync`.
unsafe impl<T> Sync for CpuLocal<T> {}

// Prevent valid instances of CpuLocal from being copied to any memory
// area outside the .cpu_local section.
impl<T> !Copy for CpuLocal<T> {}
impl<T> !Clone for CpuLocal<T> {}

// In general, it does not make any sense to send instances of CpuLocal to
// other tasks as they should live on other CPUs to make sending useful.
impl<T> !Send for CpuLocal<T> {}

// A check to ensure that the CPU-local object is never accessed before the
// initialization for all CPUs.
#[cfg(debug_assertions)]
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(debug_assertions)]
static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

impl<T> CpuLocal<T> {
    /// Initialize a CPU-local object.
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
        Self(val)
    }

    /// Get access to the underlying value with IRQs disabled.
    ///
    /// By this method, you can borrow a reference to the underlying value
    /// even if `T` is not `Sync`. Because that it is per-CPU and IRQs are
    /// disabled, no other running task can access it.
    pub fn borrow_irq_disabled(&self) -> CpuLocalDerefGuard<'_, T> {
        CpuLocalDerefGuard {
            cpu_local: self,
            _guard: disable_local(),
        }
    }

    /// Get access to the underlying value through a raw pointer.
    ///
    /// This function calculates the virtual address of the CPU-local object based on the per-
    /// cpu base address and the offset in the BSP.
    fn get(&self) -> *const T {
        // CPU-local objects should be initialized before being accessed. It should be ensured
        // by the implementation of OSTD initialization.
        #[cfg(debug_assertions)]
        debug_assert!(IS_INITIALIZED.load(Ordering::Relaxed));

        let offset = {
            let bsp_va = self as *const _ as usize;
            let bsp_base = __cpu_local_start as usize;
            // The implementation should ensure that the CPU-local object resides in the `.cpu_local`.
            debug_assert!(bsp_va + core::mem::size_of::<T>() <= __cpu_local_end as usize);

            bsp_va - bsp_base as usize
        };

        let local_base = arch::cpu::local::get_base() as usize;
        let local_va = local_base + offset;

        // A sanity check about the alignment.
        debug_assert_eq!(local_va % core::mem::align_of::<T>(), 0);

        local_va as *mut T
    }
}

// Considering a preemptive kernel, a CPU-local object may be dereferenced
// when another task tries to access it. So, we need to ensure that `T` is
// `Sync` before allowing it to be dereferenced.
impl<T: Sync> Deref for CpuLocal<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: it should be properly initialized before accesses.
        // And we do not create a mutable reference over it. It is
        // `Sync` so it can be referenced from this task.
        unsafe { &*self.get() }
    }
}

/// A guard for accessing the CPU-local object.
///
/// It ensures that the CPU-local object is accessed with IRQs
/// disabled. It is created by [`CpuLocal::borrow_irq_disabled`].
/// Do not hold this guard for a long time.
#[must_use]
pub struct CpuLocalDerefGuard<'a, T> {
    cpu_local: &'a CpuLocal<T>,
    _guard: DisabledLocalIrqGuard,
}

impl<T> Deref for CpuLocalDerefGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: it should be properly initialized before accesses.
        // And we do not create a mutable reference over it. The IRQs
        // are disabled so it can be referenced from this task.
        unsafe { &*self.cpu_local.get() }
    }
}

/// Sets the base address of the CPU-local storage for the bootstrap processor.
///
/// It should be called early to let [`crate::task::disable_preempt`] work,
/// which needs to update a CPU-local preempt lock count. Otherwise it may
/// panic when calling [`crate::task::disable_preempt`].
///
/// # Safety
///
/// It should be called only once and only on the BSP.
pub(crate) unsafe fn early_init_bsp_local_base() {
    let start_base_va = __cpu_local_start as usize as u64;
    // SAFETY: The base to be set is the start of the `.cpu_local` section,
    // where accessing the CPU-local objects have defined behaviors.
    unsafe {
        arch::cpu::local::set_base(start_base_va);
    }
}

/// Initializes the CPU local data for the bootstrap processor (BSP).
///
/// # Safety
///
/// This function can only called on the BSP, for once.
///
/// It must be guaranteed that the BSP will not access local data before
/// this function being called, otherwise copying non-constant values
/// will result in pretty bad undefined behavior.
pub(crate) unsafe fn init_on_bsp() {
    // TODO: allocate the pages for application processors and copy the
    // CPU-local objects to the allocated pages.

    #[cfg(debug_assertions)]
    {
        IS_INITIALIZED.store(true, Ordering::Relaxed);
    }
}

// These symbols are provided by the linker script.
extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

#[cfg(ktest)]
mod test {
    use core::{
        cell::RefCell,
        sync::atomic::{AtomicU8, Ordering},
    };

    use ostd_macros::ktest;

    use super::*;

    #[ktest]
    fn test_cpu_local() {
        cpu_local! {
            static FOO: RefCell<usize> = RefCell::new(1);
            static BAR: AtomicU8 = AtomicU8::new(3);
        }
        for _ in 0..10 {
            let foo_guard = FOO.borrow_irq_disabled();
            assert_eq!(*foo_guard.borrow(), 1);
            *foo_guard.borrow_mut() = 2;
            drop(foo_guard);
            for _ in 0..10 {
                assert_eq!(BAR.load(Ordering::Relaxed), 3);
                BAR.store(4, Ordering::Relaxed);
                assert_eq!(BAR.load(Ordering::Relaxed), 4);
                BAR.store(3, Ordering::Relaxed);
            }
            let foo_guard = FOO.borrow_irq_disabled();
            assert_eq!(*foo_guard.borrow(), 2);
            *foo_guard.borrow_mut() = 1;
            drop(foo_guard);
        }
    }
}
