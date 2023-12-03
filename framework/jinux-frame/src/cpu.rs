//! CPU.

use align_ext::AlignExt;
use alloc::slice;
use spin::Once;
use x86::msr::{rdmsr, wrmsr, IA32_FS_BASE};

use crate::config::PAGE_SIZE;
use crate::trap::disable_local;
use crate::vm::{paddr_to_vaddr, Vaddr, VmAllocOptions};
use core::cell::UnsafeCell;
use core::ops::Deref;

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")]{
        pub use trapframe::GeneralRegs;
        pub use crate::arch::x86::cpu::*;
    }
}

/// Defines a CPU-local variable.
///
/// All objects defined by the `cpu_local` macro will be linked to the specified `.cpu_local` segment.
/// When the OS is started, each core will copy this segment as its private storage after startup.
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
/// CpuLocal::borrow_with(&FOO, |val| {
///     println!("FOO VAL: {:?}", *val);
/// })
///
/// ```
#[macro_export]
macro_rules! cpu_local {
    // empty
    () => {};

    // multiple declarations
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        #[link_section = ".cpu_local"]
        $(#[$attr])* $vis static $name: CpuLocal<$t> = unsafe { CpuLocal::new($init) };
        $crate::cpu_local!($($rest)*);
    };

    // single declaration
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => (
        #[link_section = ".cpu_local"]
        $(#[$attr])* $vis static $name: CpuLocal<$t> = CpuLocal::new($init);
    );
}

extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

/// CPU-local objects.
///
/// A CPU-local object only gives you immutable references to the underlying value.
/// To mutate the value, one can use atomic values (e.g., `AtomicU32`) or internally mutable
/// objects (e.g., `RefCell`).
///
/// The `CpuLocal<T: Sync>` can be used directly.
/// Otherwise, the `CpuLocal<T>` must be used through `CpuLocal::borrow_with`.
///
/// On each CPU, the actual virtual address of the `CpuLocal` object is calculated as follows：
/// Object VA = base address + offset
/// FS register is used to store the base address of cpu local data, and the base addresses of
/// different cores are different to ensure the privacy of data. The offset of the linked virtual
/// address of this object from the start address of the `.cpu_local` segment is used as the
/// offset of the actual virtual address from the base address.
///
/// Compared to directly accessing an object, `CpuLocal` requires some additional overhead to
/// calculate the actual virtual address.
pub struct CpuLocal<T>(UnsafeCell<T>);

// Safety. At any given time, only one task can access the inner value T of a cpu-local variable.
unsafe impl<T> Sync for CpuLocal<T> {}

impl<T> CpuLocal<T> {
    /// Initialize CPU-local object
    ///
    /// # Safety
    ///
    /// Developer cannot construct a valid CpuLocal object arbitrarily. Create a valid CpuLocal
    /// object using the `cpu_local` macro instead of directly using the `new` method.
    #[allow(clippy::missing_safety_doc)]
    pub const unsafe fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    /// Borrow an immutable reference to the underlying value and feed it to a closure.
    ///
    /// During the execution of the closure, local IRQs are disabled. This ensures that
    /// the CPU-local object is only accessed by the current task or IRQ handler.
    /// As local IRQs are disabled, one should keep the closure as short as possible.
    pub fn borrow_with<U, F: FnOnce(&T) -> U>(this: &Self, f: F) -> U {
        // FIXME: implement disable preemption
        // Disable interrupts when accessing cpu-local variable
        let _guard = disable_local();
        // Safety. Now that the local IRQs are disabled, this CPU-local object can only be
        // accessed by the current task/thread. So it is safe to get its immutable reference
        // regardless of whether `T` implements `Sync` or not.
        let val_ref = unsafe { this.do_borrow() };
        f(val_ref)
    }

    unsafe fn do_borrow(&self) -> &T {
        &*self.data().get()
    }

    /// Calculates the actual virtual address of an object and returns the object.
    fn data(&self) -> &UnsafeCell<T> {
        let base_va = unsafe { rdmsr(IA32_FS_BASE) };
        let self_va = self as *const _ as usize;
        let self_offset = self_va - __cpu_local_start as usize;
        let data_va = (base_va as usize + self_offset) as Vaddr;
        unsafe { &*(data_va as *const UnsafeCell<T>) }
    }
}

impl<T: Sync> Deref for CpuLocal<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.data().get() }
    }
}

cpu_local! {
    pub static CPUID: Once<u32> = Once::new();
}

pub fn init(cpu_id: u32) {
    let start_base_va = __cpu_local_start as usize;
    let end_base_va = __cpu_local_end as usize;
    let nbytes = (end_base_va - start_base_va).align_up(PAGE_SIZE);
    if cpu_id == 0 {
        // During the initialization of the CpuLocal object in BSP, the frame allocator has
        // not been initialized yet, thus a reserved memory segment is used as its local storage.
        extern "C" {
            fn __bsp_local_start();
        }
        unsafe {
            // safety: Both [__cpu_local_start, __cpu_local_end) and [__bsp_local_start, __bsp_local_end)
            // are reserved areas used for storing cpulocal data.
            core::ptr::copy(
                start_base_va as *const u8,
                __bsp_local_start as usize as *mut u8,
                nbytes,
            );
            // safety: FS register is not used for any other purpose.
            wrmsr(IA32_FS_BASE, __bsp_local_start as u64);
        }
        (*CPUID).call_once(|| cpu_id);
        return;
    }
    let local_segment = VmAllocOptions::new(nbytes / PAGE_SIZE)
        .is_contiguous(true)
        .need_dealloc(false)
        .alloc_contiguous()
        .unwrap();
    // safety: [__cpu_local_start, __cpu_local_end) is reserved area used for storing cpulocal data.
    local_segment
        .writer()
        .write(&mut unsafe { slice::from_raw_parts(start_base_va as *const u8, nbytes) }.into());
    // safety: FS register is not used for any other purpose.
    unsafe {
        wrmsr(
            IA32_FS_BASE,
            paddr_to_vaddr(local_segment.start_paddr()) as u64,
        );
    }
    (*CPUID).call_once(|| cpu_id);
}
