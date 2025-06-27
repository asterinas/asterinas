// SPDX-License-Identifier: MPL-2.0

//! CPU local storage.
//!
//! This module provides a mechanism to define CPU-local objects. Users can
//! define a statically-allocated CPU-local object by the macro
//! [`crate::cpu_local!`], or allocate a dynamically-allocated CPU-local
//! object with the function `osdk_heap_allocator::alloc_cpu_local`.
//!
//! The mechanism for statically-allocated CPU-local objects exploits the fact
//! that constant values of non-[`Copy`] types can be bitwise copied. For
//! example, a [`Option<T>`] object, though being not [`Copy`], have a constant
//! constructor [`Option::None`] that produces a value that can be bitwise
//! copied to create a new instance. [`alloc::sync::Arc`] however, don't have
//! such a constructor, and thus cannot be directly used as a statically-
//! allocated CPU-local object. Wrapping it in a type that has a constant
//! constructor, like [`Option<T>`], can make it statically-allocated CPU-local.
//!
//! # Implementation
//!
//! These APIs are implemented by the methods as follows:
//! 1. For statically-allocated CPU-local objects, we place them in a special
//!    section `.cpu_local`. The bootstrap processor (BSP) uses the objects
//!    linked in this section, and these objects are copied to dynamically
//!    allocated local storage of each application processors (AP) during the
//!    initialization process.
//! 2. For dynamically-allocated CPU-local objects, we prepare a fixed-size
//!    chunk for each CPU. These per-CPU memory chunks are laid out contiguously
//!    in memory in the order of the CPU IDs. A dynamically-allocated CPU-local
//!    object can be allocated by occupying the same offset in each per-CPU
//!    memory chunk.

// This module also, provide CPU-local cell objects that have inner mutability.
//
// The difference between statically-allocated CPU-local objects (defined by
// [`crate::cpu_local!`]) and CPU-local cell objects (defined by
// [`crate::cpu_local_cell!`]) is that the CPU-local objects can be shared
// across CPUs. While through a CPU-local cell object you can only access the
// value on the current CPU, therefore enabling inner mutability without locks.

mod cell;
mod dyn_cpu_local;
mod static_cpu_local;

pub(crate) mod single_instr;

use core::{alloc::Layout, marker::PhantomData, ops::Deref};

use align_ext::AlignExt;
pub use cell::CpuLocalCell;
pub use dyn_cpu_local::DynCpuLocalChunk;
use dyn_cpu_local::DynamicStorage;
use spin::Once;
use static_cpu_local::StaticStorage;

use super::CpuId;
use crate::{
    mm::{frame::allocator, paddr_to_vaddr, Paddr, PAGE_SIZE},
    trap::irq::DisabledLocalIrqGuard,
};

/// Dynamically-allocated CPU-local objects.
pub type DynamicCpuLocal<T> = CpuLocal<T, DynamicStorage<T>>;

/// Statically-allocated CPU-local objects.
pub type StaticCpuLocal<T> = CpuLocal<T, static_cpu_local::StaticStorage<T>>;

// These symbols are provided by the linker script.
extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

/// A trait to abstract any type that can be used as a slot for a CPU-local
/// variable of type `T`.
///
/// Each slot provides the memory space for storing `num_cpus` instances
/// of type `T`.
///
/// # Safety
///
/// The implementor must ensure that the returned pointer refers to the
/// variable on the correct CPU.
pub unsafe trait AnyStorage<T> {
    /// Gets the `const` pointer for the object on the current CPU.
    fn get_ptr_on_current(&self, guard: &DisabledLocalIrqGuard) -> *const T;

    /// Gets the `const` pointer for the object on a target CPU.
    fn get_ptr_on_target(&self, cpu: CpuId) -> *const T;

    /// Gets the `mut` pointer for the object on a target CPU.
    ///
    /// This method is intended for use when initializing or dropping the storage.
    fn get_mut_ptr_on_target(&mut self, cpu: CpuId) -> *mut T;
}

/// A CPU-local variable for type `T`, backed by a storage of type `S`.
///
/// CPU-local objects are instantiated once per CPU core. They can be shared to
/// other cores. In the context of a preemptible kernel task, when holding the
/// reference to the inner object, the object is always the one in the original
/// core (when the reference is created), no matter which core the code is
/// currently running on.
pub struct CpuLocal<T, S: AnyStorage<T>> {
    storage: S,
    phantom: PhantomData<T>,
}

impl<T: 'static, S: AnyStorage<T>> CpuLocal<T, S> {
    /// Gets access to the underlying value on the current CPU with a
    /// provided IRQ guard.
    ///
    /// By this method, you can borrow a reference to the underlying value
    /// on the current CPU even if `T` is not `Sync`.
    pub fn get_with<'a>(
        &'a self,
        guard: &'a DisabledLocalIrqGuard,
    ) -> CpuLocalDerefGuard<'a, T, S> {
        CpuLocalDerefGuard {
            cpu_local: self,
            guard,
        }
    }
}

impl<T: 'static + Sync, S: AnyStorage<T>> CpuLocal<T, S> {
    /// Gets access to the CPU-local value on a specific CPU.
    ///
    /// This allows the caller to access CPU-local data from a remote CPU,
    /// so the data type must be `Sync`.
    pub fn get_on_cpu(&self, target_cpu_id: CpuId) -> &T {
        let ptr = self.storage.get_ptr_on_target(target_cpu_id);
        // SAFETY: `ptr` represents CPU-local data on a remote CPU. It
        // contains valid data, the type is `Sync`, and no one will mutably
        // borrow it, so creating an immutable borrow here is valid.
        unsafe { &*ptr }
    }
}

/// A guard for accessing the CPU-local object.
///
/// It ensures that the CPU-local object is accessed with IRQs disabled.
/// It is created by [`CpuLocal::get_with`].
#[must_use]
pub struct CpuLocalDerefGuard<'a, T: 'static, S: AnyStorage<T>> {
    cpu_local: &'a CpuLocal<T, S>,
    guard: &'a DisabledLocalIrqGuard,
}

impl<'a, T: 'static, S: AnyStorage<T>> Deref for CpuLocalDerefGuard<'a, T, S> {
    type Target = T;

    fn deref(&self) -> &'a Self::Target {
        is_used::debug_set_true();

        let ptr = self.cpu_local.storage.get_ptr_on_current(self.guard);
        // SAFETY: `ptr` represents CPU-local data on the current CPU. It
        // contains valid data, only the current task can reference the data
        // (due to `self.guard`), and no one will mutably borrow it, so
        // creating an immutable borrow here is valid.
        unsafe { &*ptr }
    }
}

// SAFETY: Although multiple tasks may access the inner value `T` of a CPU-local
// variable at different times, only one task can access it at any given moment.
// We guarantee it by disabling the reference to the inner value, or turning off
// preemptions when creating the reference. Therefore, if `T` is `Send`, marking
// `CpuLocal<T, S>` with `Sync` and `Send` only safely transfer ownership of the
// entire `T` instance between tasks.
unsafe impl<T: Send + 'static, S: AnyStorage<T>> Sync for CpuLocal<T, S> {}
unsafe impl<T: Send + 'static> Send for CpuLocal<T, DynamicStorage<T>> {}

// Implement `!Copy` and `!Clone` for `CpuLocal` to ensure memory safety:
// - Prevent valid instances of `CpuLocal<T, StaticStorage<T>>` from being copied
// to any memory areas outside the `.cpu_local` section.
// - Prevent multiple valid instances of `CpuLocal<T, DynamicStorage<T>>` from
// referring to the same CPU-local object, avoiding double deallocation.
impl<T: 'static, S: AnyStorage<T>> !Copy for CpuLocal<T, S> {}
impl<T: 'static, S: AnyStorage<T>> !Clone for CpuLocal<T, S> {}

// In general, it does not make any sense to send instances of static `CpuLocal`
// to other tasks as they should live on other CPUs to make sending useful.
impl<T: 'static> !Send for CpuLocal<T, StaticStorage<T>> {}

/// The static CPU-local areas for APs.
static CPU_LOCAL_STORAGES: Once<&'static [Paddr]> = Once::new();

/// Copies the static CPU-local data on the bootstrap processor (BSP)
/// for application processors (APs).
///
/// # Safety
///
/// This function must be called in the boot context of the BSP, at a time
/// when the APs have not yet booted.
///
/// The CPU-local data on the BSP must not be used before calling this
/// function to copy it for the APs. Otherwise, the copied data will
/// contain non-constant (also non-`Copy`) data, resulting in undefined
/// behavior when it's loaded on the APs.
///
/// The caller must ensure that the `num_cpus` matches the number of all
/// CPUs that will access the CPU-local storage.
pub(crate) unsafe fn copy_bsp_for_ap(num_cpus: usize) {
    let num_aps = num_cpus - 1; // BSP does not need allocated storage.
    if num_aps == 0 {
        return;
    }

    // Allocate a region to store the pointers to the CPU-local storage segments.
    let res = {
        let size = core::mem::size_of::<Paddr>()
            .checked_mul(num_aps)
            .unwrap()
            .align_up(PAGE_SIZE);
        let addr =
            allocator::early_alloc(Layout::from_size_align(size, PAGE_SIZE).unwrap()).unwrap();
        let ptr = paddr_to_vaddr(addr) as *mut Paddr;

        // SAFETY: The memory is properly allocated. We exclusively own it. So it's valid to write.
        unsafe {
            core::ptr::write_bytes(ptr as *mut u8, 0, size);
        }
        // SAFETY: The memory is properly allocated and initialized. We exclusively own it. We
        // never deallocate it so it lives for '`static'. So we can create a mutable slice on it.
        unsafe { core::slice::from_raw_parts_mut(ptr, num_aps) }
    };

    let bsp_base_va = __cpu_local_start as usize;
    let bsp_end_va = __cpu_local_end as usize;

    // Allocate the CPU-local storage segments for APs.
    for res_addr_mut in res.iter_mut() {
        let nbytes = (bsp_end_va - bsp_base_va).align_up(PAGE_SIZE);
        let ap_pages =
            allocator::early_alloc(Layout::from_size_align(nbytes, PAGE_SIZE).unwrap()).unwrap();
        let ap_pages_ptr = paddr_to_vaddr(ap_pages) as *mut u8;

        // SAFETY:
        // 1. The source is valid to read because it has not been used before,
        //    so it contains only constants.
        // 2. The destination is valid to write because it is just allocated.
        // 3. The memory is aligned because the alignment of `u8` is 1.
        // 4. The two memory regions do not overlap because allocated memory
        //    regions never overlap with the kernel data.
        unsafe {
            core::ptr::copy_nonoverlapping(bsp_base_va as *const u8, ap_pages_ptr, nbytes);
        }

        *res_addr_mut = ap_pages;
    }

    is_used::debug_assert_false();

    assert!(!CPU_LOCAL_STORAGES.is_completed());
    CPU_LOCAL_STORAGES.call_once(|| res);
}

/// Gets the pointer to the static CPU-local storage for the given AP.
///
/// # Panics
///
/// This method will panic if the `cpu_id` does not represent an AP or the AP's CPU-local storage
/// has not been allocated.
pub(crate) fn get_ap(cpu_id: CpuId) -> Paddr {
    let offset = cpu_id
        .as_usize()
        .checked_sub(1)
        .expect("The BSP does not have allocated CPU-local storage");

    let paddr = CPU_LOCAL_STORAGES
        .get()
        .expect("No CPU-local storage has been allocated")[offset];
    assert_ne!(
        paddr,
        0,
        "The CPU-local storage for CPU {} is not allocated",
        cpu_id.as_usize(),
    );
    paddr
}

mod is_used {
    //! This module tracks whether any statically-allocated CPU-local
    //! variables are used.
    //!
    //! [`copy_bsp_for_ap`] copies the CPU local data from the BSP
    //! to the APs, so it requires as a safety condition that the
    //! CPU-local data has not been accessed before the copy. This
    //! module provides utilities to check if the safety condition
    //! is met, but only if debug assertions are enabled.
    //!
    //! [`copy_bsp_for_ap`]: super::copy_bsp_for_ap

    cfg_if::cfg_if! {
        if #[cfg(debug_assertions)] {
            use core::sync::atomic::{AtomicBool, Ordering};

            static IS_USED: AtomicBool = AtomicBool::new(false);

            pub fn debug_set_true() {
                IS_USED.store(true, Ordering::Relaxed);
            }

            pub fn debug_assert_false() {
                debug_assert!(!IS_USED.load(Ordering::Relaxed));
            }
        } else {
            pub fn debug_set_true() {}

            pub fn debug_assert_false() {}
        }
    }
}

#[cfg(ktest)]
mod test {
    use core::cell::RefCell;

    use ostd_macros::ktest;

    #[ktest]
    fn test_cpu_local() {
        crate::cpu_local! {
            static FOO: RefCell<usize> = RefCell::new(1);
        }
        let irq_guard = crate::trap::irq::disable_local();
        let foo_guard = FOO.get_with(&irq_guard);
        assert_eq!(*foo_guard.borrow(), 1);
        *foo_guard.borrow_mut() = 2;
        assert_eq!(*foo_guard.borrow(), 2);
        drop(foo_guard);
    }

    #[ktest]
    fn test_cpu_local_cell() {
        crate::cpu_local_cell! {
            static BAR: usize = 3;
        }
        let _guard = crate::trap::irq::disable_local();
        assert_eq!(BAR.load(), 3);
        BAR.store(4);
        assert_eq!(BAR.load(), 4);
    }
}
