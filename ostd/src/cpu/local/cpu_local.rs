// SPDX-License-Identifier: MPL-2.0

//! The CPU-local variable implementation.

use core::{marker::Sync, ops::Deref};

use super::{__cpu_local_end, __cpu_local_start};
use crate::{
    arch,
    trap::{self, DisabledLocalIrqGuard},
};

/// Defines a CPU-local variable.
///
/// The accessors of the CPU-local variables are defined with [`CpuLocal`].
///
/// You can get the reference to the inner object by calling [`deref`]. But
/// it is worth noting that the object is always the one in the original core
/// when the reference is created. Use [`CpuLocal::borrow_irq_disabled`] if
/// this is not expected, or if the inner type can't be shared across CPUs.
///
/// # Example
///
/// ```rust
/// use ostd::{cpu_local, sync::SpinLock};
/// use core::sync::atomic::{AtomicU32, Ordering};
///
/// cpu_local! {
///     static FOO: AtomicU32 = AtomicU32::new(1);
///     pub static BAR: SpinLock<usize> = SpinLock::new(2);
/// }
///
/// fn not_an_atomic_function() {
///     let ref_of_foo = FOO.deref();
///     // Note that the value of `FOO` here doesn't necessarily equal to the value
///     // of `FOO` of exactly the __current__ CPU. Since that task may be preempted
///     // and moved to another CPU since `ref_of_foo` is created.
///     let val_of_foo = ref_of_foo.load(Ordering::Relaxed);
///     println!("FOO VAL: {}", val_of_foo);
///
///     let bar_guard = BAR.lock_irq_disabled();
///     // Here the value of `BAR` is always the one in the __current__ CPU since
///     // interrupts are disabled and we do not explicitly yield execution here.
///     let val_of_bar = *bar_guard;
///     println!("BAR VAL: {}", val_of_bar);
/// }
/// ```
#[macro_export]
macro_rules! cpu_local {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            #[link_section = ".cpu_local"]
            $(#[$attr])* $vis static $name: $crate::cpu::local::CpuLocal<$t> = {
                let val = $init;
                // SAFETY: The per-CPU variable instantiated is statically
                // stored in the special `.cpu_local` section.
                unsafe {
                    $crate::cpu::local::CpuLocal::__new(val)
                }
            };
        )*
    };
}

/// CPU-local objects.
///
/// CPU-local objects are instanciated once per CPU core. They can be shared to
/// other cores. In the context of a preemptible kernel task, when holding the
/// reference to the inner object, the object is always the one in the original
/// core (when the reference is created), no matter which core the code is
/// currently running on.
///
/// For the difference between [`CpuLocal`] and [`super::CpuLocalCell`], see
/// [`super`].
pub struct CpuLocal<T: 'static>(T);

impl<T: 'static> CpuLocal<T> {
    /// Creates a new CPU-local object.
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
    /// disabled, no other running tasks can access it.
    pub fn borrow_irq_disabled(&'static self) -> CpuLocalDerefGuard<'_, T> {
        CpuLocalDerefGuard {
            cpu_local: self,
            _guard: InnerGuard::Created(trap::disable_local()),
        }
    }

    /// Get access to the underlying value with a provided guard.
    ///
    /// Similar to [`CpuLocal::borrow_irq_disabled`], but you can provide
    /// a guard to disable IRQs if you already have one.
    pub fn borrow_with<'a>(
        &'static self,
        guard: &'a DisabledLocalIrqGuard,
    ) -> CpuLocalDerefGuard<'a, T> {
        CpuLocalDerefGuard {
            cpu_local: self,
            _guard: InnerGuard::Provided(guard),
        }
    }

    /// Get access to the underlying value through a raw pointer.
    ///
    /// This function calculates the virtual address of the CPU-local object
    /// based on the CPU-local base address and the offset in the BSP.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the reference to `self` is static.
    unsafe fn as_ptr(&self) -> *const T {
        super::has_init::assert_true();

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

// SAFETY: At any given time, only one task can access the inner value `T` of a
// CPU-local variable if `T` is not `Sync`. We guarentee it by disabling the
// reference to the inner value, or turning off preemptions when creating
// the reference.
unsafe impl<T: 'static> Sync for CpuLocal<T> {}

// Prevent valid instances of `CpuLocal` from being copied to any memory areas
// outside the `.cpu_local` section.
impl<T: 'static> !Copy for CpuLocal<T> {}
impl<T: 'static> !Clone for CpuLocal<T> {}

// In general, it does not make any sense to send instances of `CpuLocal` to
// other tasks as they should live on other CPUs to make sending useful.
impl<T: 'static> !Send for CpuLocal<T> {}

// For `Sync` types, we can create a reference over the inner type and allow
// it to be shared across CPUs. So it is sound to provide a `Deref`
// implementation. However it is up to the caller if sharing is desired.
impl<T: 'static + Sync> Deref for CpuLocal<T> {
    type Target = T;

    /// Note that the reference to the inner object remains to the same object
    /// accessed on the original CPU where the reference is created. If this
    /// is not expected, turn off preemptions.
    fn deref(&self) -> &Self::Target {
        // SAFETY: it should be properly initialized before accesses.
        // And we do not create a mutable reference over it. It is
        // `Sync` so it can be referenced from this task. Here dereferencing
        // from non-static instances is not feasible since no one can create
        // a non-static instance of `CpuLocal`.
        unsafe { &*self.as_ptr() }
    }
}

/// A guard for accessing the CPU-local object.
///
/// It ensures that the CPU-local object is accessed with IRQs disabled.
/// It is created by [`CpuLocal::borrow_irq_disabled`] or
/// [`CpuLocal::borrow_with`]. Do not hold this guard for a longtime.
#[must_use]
pub struct CpuLocalDerefGuard<'a, T: 'static> {
    cpu_local: &'static CpuLocal<T>,
    _guard: InnerGuard<'a>,
}

enum InnerGuard<'a> {
    #[allow(dead_code)]
    Created(DisabledLocalIrqGuard),
    #[allow(dead_code)]
    Provided(&'a DisabledLocalIrqGuard),
}

impl<T: 'static> Deref for CpuLocalDerefGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: it should be properly initialized before accesses.
        // And we do not create a mutable reference over it. The IRQs
        // are disabled so it can only be referenced from this task.
        unsafe { &*self.cpu_local.as_ptr() }
    }
}
