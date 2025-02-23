// SPDX-License-Identifier: MPL-2.0

//! CPU local storage.
//!
//! This module provides a mechanism to define CPU-local objects, by the macro
//! [`crate::cpu_local!`].
//!
//! Such a mechanism exploits the fact that constant values of non-[`Copy`]
//! types can be bitwise copied. For example, a [`Option<T>`] object, though
//! being not [`Copy`], have a constant constructor [`Option::None`] that
//! produces a value that can be bitwise copied to create a new instance.
//! [`alloc::sync::Arc`] however, don't have such a constructor, and thus cannot
//! be directly used as a CPU-local object. Wrapping it in a type that has a
//! constant constructor, like [`Option<T>`], can make it CPU-local.
//!
//! # Implementation
//!
//! These APIs are implemented by placing the CPU-local objects in a special
//! section `.cpu_local`. The bootstrap processor (BSP) uses the objects linked
//! in this section, and these objects are copied to dynamically allocated
//! local storage of each application processors (AP) during the initialization
//! process.

// This module also, provide CPU-local cell objects that have inner mutability.
//
// The difference between CPU-local objects (defined by [`crate::cpu_local!`])
// and CPU-local cell objects (defined by [`crate::cpu_local_cell!`]) is that
// the CPU-local objects can be shared across CPUs. While through a CPU-local
// cell object you can only access the value on the current CPU, therefore
// enabling inner mutability without locks.

mod cell;
mod cpu_local;

pub(crate) mod single_instr;

use alloc::{boxed::Box, vec::Vec};

pub use cell::CpuLocalCell;
pub use cpu_local::{CpuLocal, CpuLocalDerefGuard};
use spin::Once;

use crate::mm::{frame::Segment, kspace::KernelMeta, paddr_to_vaddr, FrameAllocOptions, PAGE_SIZE};

// These symbols are provided by the linker script.
extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

/// CPU-local storages for APs.
static CPU_LOCAL_STORAGES: Once<Box<[Segment<KernelMeta>]>> = Once::new();

/// Copies the CPU-local data on the bootstrap processor (BSP)
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
pub(crate) unsafe fn copy_bsp_for_ap(num_cpus: usize) -> &'static [Segment<KernelMeta>] {
    let bsp_base_va = __cpu_local_start as usize;
    let bsp_end_va = __cpu_local_end as usize;

    let mut cpu_local_storages = Vec::with_capacity(num_cpus.saturating_sub(1));
    for _ in 1..num_cpus {
        let nbytes = bsp_end_va - bsp_base_va;

        let ap_pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(nbytes.div_ceil(PAGE_SIZE), |_| KernelMeta)
            .unwrap();
        let ap_pages_ptr = paddr_to_vaddr(ap_pages.start_paddr()) as *mut u8;

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

        cpu_local_storages.push(ap_pages);
    }

    assert!(!CPU_LOCAL_STORAGES.is_completed());
    let cpu_local_storages =
        CPU_LOCAL_STORAGES.call_once(move || cpu_local_storages.into_boxed_slice());

    is_used::debug_assert_false();

    cpu_local_storages
}

mod is_used {
    //! This module tracks whether any CPU-local variables are used.
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
        let irq_guard = crate::trap::disable_local();
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
        let _guard = crate::trap::disable_local();
        assert_eq!(BAR.load(), 3);
        BAR.store(4);
        assert_eq!(BAR.load(), 4);
    }
}
