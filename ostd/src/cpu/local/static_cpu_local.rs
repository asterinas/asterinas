// SPDX-License-Identifier: MPL-2.0

//! Statically-allocated CPU-local objects.

use core::marker::PhantomData;

use super::{AnyStorage, CpuLocal, __cpu_local_end, __cpu_local_start};
use crate::{arch, cpu::CpuId, trap::irq::DisabledLocalIrqGuard};

/// Defines a statically-allocated CPU-local variable.
///
/// The accessors of the CPU-local variables are defined with [`CpuLocal`].
///
/// You can get the reference to the inner object on one CPU by calling
/// [`CpuLocal::get_on_cpu`]. Also if you intend to access the inner object
/// on the current CPU, you can use [`CpuLocal::get_with`]. The latter
/// accessors can be used even if the inner object is not `Sync`.
///
/// # Example
///
/// ```rust
/// use ostd::{cpu_local, cpu::PinCurrentCpu, task::disable_preempt, trap};
/// use core::{sync::atomic::{AtomicU32, Ordering}, cell::Cell};
///
/// cpu_local! {
///     static FOO: AtomicU32 = AtomicU32::new(1);
///     pub static BAR: Cell<usize> = Cell::new(2);
/// }
///
/// fn not_an_atomic_function() {
///     let preempt_guard = disable_preempt();
///     let ref_of_foo = FOO.get_on_cpu(preempt_guard.current_cpu());
///     let val_of_foo = ref_of_foo.load(Ordering::Relaxed);
///     println!("FOO VAL: {}", val_of_foo);
///
///     let irq_guard = trap::irq::disable_local();
///     let bar_guard = BAR.get_with(&irq_guard);
///     let val_of_bar = bar_guard.get();
///     println!("BAR VAL: {}", val_of_bar);
/// }
/// ```
#[macro_export]
macro_rules! cpu_local {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            #[link_section = ".cpu_local"]
            $(#[$attr])* $vis static $name: $crate::cpu::local::StaticCpuLocal<$t> = {
                let val = $init;
                // SAFETY: The per-CPU variable instantiated is statically
                // stored in the special `.cpu_local` section.
                unsafe {
                    $crate::cpu::local::CpuLocal::__new_static(val)
                }
            };
        )*
    };
}

/// A static storage for a CPU-local variable of type `T`.
///
/// Such a CPU-local storage is not intended to be allocated directly.
/// Use the `cpu_local` macro instead.
pub struct StaticStorage<T: 'static>(T);

impl<T: 'static> StaticStorage<T> {
    /// Gets access to the underlying value through a raw pointer.
    ///
    /// This method is safe, but using the returned pointer will be unsafe.
    fn as_ptr(&self) -> *const T {
        super::is_used::debug_set_true();

        let offset = self.get_offset();

        let local_base = arch::cpu::local::get_base() as usize;
        let local_va = local_base + offset;

        // A sanity check about the alignment.
        debug_assert_eq!(local_va % core::mem::align_of::<T>(), 0);

        local_va as *const T
    }

    /// Gets the offset of the CPU-local object in the CPU-local area.
    fn get_offset(&self) -> usize {
        let bsp_va = self as *const _ as usize;
        let bsp_base = __cpu_local_start as usize;
        // The implementation should ensure that the CPU-local object resides in the `.cpu_local`.
        debug_assert!(bsp_va + core::mem::size_of::<T>() <= __cpu_local_end as usize);

        bsp_va - bsp_base
    }
}

unsafe impl<T: 'static> AnyStorage<T> for StaticStorage<T> {
    fn get_ptr_on_current(&self, _guard: &DisabledLocalIrqGuard) -> *const T {
        self.as_ptr()
    }

    fn get_ptr_on_target(&self, cpu_id: CpuId) -> *const T {
        super::is_used::debug_set_true();

        let cpu_id = cpu_id.as_usize();

        // If on the BSP, just use the statically linked storage.
        if cpu_id == 0 {
            return &self.0 as *const T;
        }

        let base = {
            // SAFETY: At this time we have a non-BSP `CpuId`, which means that
            // `init_cpu_nums` must have been called, so `copy_bsp_for_ap` must
            // also have been called (see the implementation of `cpu::init_on_bsp`),
            // so `CPU_LOCAL_STORAGES` must already be initialized.
            let storages = unsafe { super::CPU_LOCAL_STORAGES.get_unchecked() };
            // SAFETY: `cpu_id` is guaranteed to be in range because the type
            // invariant of `CpuId`.
            let storage = unsafe { *storages.get_unchecked(cpu_id - 1) };
            crate::mm::paddr_to_vaddr(storage)
        };

        let offset = self.get_offset();
        (base + offset) as *const T
    }

    fn get_mut_ptr_on_target(&mut self, _: CpuId) -> *mut T {
        // `StaticStorage<T>` does not support `get_mut_ptr_on_target`, because
        // statically-allocated CPU-local objects do not require per-CPU initialization.
        panic!("Can't get the mutable pointer of StaticStorage<T> on a target CPU.");
    }
}

impl<T: 'static> CpuLocal<T, StaticStorage<T>> {
    /// Creates a new statically-allocated CPU-local object.
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
    pub const unsafe fn __new_static(val: T) -> Self {
        Self {
            storage: StaticStorage(val),
            phantom: PhantomData,
        }
    }

    /// Gets access to the underlying value through a raw pointer.
    ///
    /// This method is safe, but using the returned pointer will be unsafe.
    pub(crate) fn as_ptr(&self) -> *const T {
        self.storage.as_ptr()
    }
}
