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

use alloc::vec::Vec;

use align_ext::AlignExt;
pub use cell::CpuLocalCell;
pub use cpu_local::{CpuLocal, CpuLocalDerefGuard};
use spin::Once;

use crate::{
    arch,
    mm::{
        paddr_to_vaddr,
        page::{self, meta::KernelMeta, ContPages},
        PAGE_SIZE,
    },
};

// These symbols are provided by the linker script.
extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

/// Sets the base address of the CPU-local storage for the bootstrap processor.
///
/// It should be called early to let [`crate::task::disable_preempt`] work,
/// which needs to update a CPU-local preemption info. Otherwise it may
/// panic when calling [`crate::task::disable_preempt`]. It is needed since
/// heap allocations need to disable preemption, which would happen in the
/// very early stage of the kernel.
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

/// The BSP initializes the CPU-local areas for APs.
static CPU_LOCAL_STORAGES: Once<Vec<ContPages<KernelMeta>>> = Once::new();

/// Initializes the CPU local data for the bootstrap processor (BSP).
///
/// # Safety
///
/// This function can only called on the BSP, for once.
///
/// It must be guaranteed that the BSP will not access local data before
/// this function being called, otherwise copying non-constant values
/// will result in pretty bad undefined behavior.
pub unsafe fn init_on_bsp() {
    let bsp_base_va = __cpu_local_start as usize;
    let bsp_end_va = __cpu_local_end as usize;

    let num_cpus = super::num_cpus();

    let mut cpu_local_storages = Vec::with_capacity(num_cpus - 1);
    for _ in 1..num_cpus {
        let ap_pages = {
            let nbytes = (bsp_end_va - bsp_base_va).align_up(PAGE_SIZE);
            page::allocator::alloc_contiguous(nbytes, |_| KernelMeta::default()).unwrap()
        };
        let ap_pages_ptr = paddr_to_vaddr(ap_pages.start_paddr()) as *mut u8;

        // SAFETY: The BSP has not initialized the CPU-local area, so the objects in
        // in the `.cpu_local` section can be bitwise bulk copied to the AP's local
        // storage. The destination memory is allocated so it is valid to write to.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bsp_base_va as *const u8,
                ap_pages_ptr,
                bsp_end_va - bsp_base_va,
            );
        }

        cpu_local_storages.push(ap_pages);
    }

    CPU_LOCAL_STORAGES.call_once(|| cpu_local_storages);

    arch::cpu::local::set_base(bsp_base_va as u64);

    has_init::set_true();
}

/// Initializes the CPU local data for the application processor (AP).
///
/// # Safety
///
/// This function can only called on the AP.
pub unsafe fn init_on_ap(cpu_id: u32) {
    let ap_pages = CPU_LOCAL_STORAGES
        .get()
        .unwrap()
        .get(cpu_id as usize - 1)
        .unwrap();

    let ap_pages_ptr = paddr_to_vaddr(ap_pages.start_paddr()) as *mut u32;

    // SAFETY: the memory will be dedicated to the AP. And we are on the AP.
    unsafe {
        arch::cpu::local::set_base(ap_pages_ptr as u64);
    }

    crate::task::reset_preempt_info();
}

mod has_init {
    //! This module is used to detect the programming error of using the CPU-local
    //! mechanism before it is initialized. Such bugs have been found before and we
    //! do not want to repeat this error again. This module is only incurs runtime
    //! overhead if debug assertions are enabled.
    cfg_if::cfg_if! {
        if #[cfg(debug_assertions)] {
            use core::sync::atomic::{AtomicBool, Ordering};

            static IS_INITIALIZED: AtomicBool = AtomicBool::new(false);

            pub fn assert_true() {
                debug_assert!(IS_INITIALIZED.load(Ordering::Relaxed));
            }

            pub fn set_true() {
                IS_INITIALIZED.store(true, Ordering::Relaxed);
            }
        } else {
            pub fn assert_true() {}

            pub fn set_true() {}
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
