// SPDX-License-Identifier: MPL-2.0

//! CPU.

use alloc::{slice, vec::Vec};
use core::{cell::UnsafeCell, ops::Deref};

use align_ext::AlignExt;
use bitvec::{order::Lsb0, slice::IterOnes, vec::BitVec};
use spin::Once;

use crate::{
    smp::CPUNUM,
    trap::disable_local,
    vm::{paddr_to_vaddr, Vaddr, VmAllocOptions, PAGE_SIZE},
};

cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")]{
        pub use trapframe::GeneralRegs;
        pub use crate::arch::x86::cpu::*;
    }
}

/// Returns the number of CPUs.
pub fn num_cpus() -> u32 {
    *CPUNUM.get().unwrap()
}

/// Returns the ID of this CPU.
pub fn this_cpu() -> u32 {
    *CPUID.get().unwrap()
}

#[derive(Default)]
pub struct CpuSet {
    bitset: BitVec,
}

impl CpuSet {
    pub fn new_full() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, true);
        Self { bitset }
    }

    pub fn new_empty() -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, false);
        Self { bitset }
    }

    pub fn from_cpu_id(cpu_id: u32) -> Self {
        let num_cpus = num_cpus();
        let mut bitset = BitVec::with_capacity(num_cpus as usize);
        bitset.resize(num_cpus as usize, false);
        bitset.set(cpu_id as usize, true);
        Self { bitset }
    }

    pub fn add(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, true);
    }

    pub fn add_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.add(cpu_id)
        }
    }

    pub fn add_all(&mut self) {
        self.bitset.fill(true);
    }

    pub fn remove(&mut self, cpu_id: u32) {
        self.bitset.set(cpu_id as usize, false);
    }

    pub fn remove_from_vec(&mut self, cpu_ids: Vec<u32>) {
        for cpu_id in cpu_ids {
            self.remove(cpu_id);
        }
    }

    pub fn clear(&mut self) {
        self.bitset.fill(false);
    }

    pub fn contains(&self, cpu_id: u32) -> bool {
        self.bitset.get(cpu_id as usize).as_deref() == Some(&true)
    }

    pub fn iter(&self) -> IterOnes<'_, usize, Lsb0> {
        self.bitset.iter_ones()
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
        $(#[$attr])* $vis static $name: $crate::cpu::CpuLocal<$t> = unsafe { $crate::cpu::CpuLocal::new($init) };
        $crate::cpu_local!($($rest)*);
    };
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
        // Disable interrupts when accessing cpu-local variable
        let _guard = disable_local();
        // Safety: Now that the local IRQs are disabled, this CPU-local object can only be
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
        let base_va = get_cpu_local_base_addr();
        let self_va = self as *const _ as usize;
        let self_offset = self_va - __cpu_local_start as usize;
        let data_va = (base_va as usize + self_offset) as Vaddr;
        // Safety: This data ensures that the initialization state has not been
        // modified before memory copying, so it can be safely dereferenced to type `T`.
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
    /// Ensure that the CPU with CPUID 0 is the boot CPU, and the others are application CPUs.
    pub static CPUID: Once<u32> = Once::new();
}

// TODO: Support automatic initialization of data on first access, just like `lazy_static` does.
fn prepare_cpu_local_data(cpu_id: u32) {
    CPUID.call_once(|| cpu_id);
}

/// Initializes the local CPU data for the bootstrap processor (BSP).
/// During the initialization of the local data in BSP, the frame allocator has
/// not been initialized yet, thus a reserved memory segment is used as its local storage.
///
/// # Safety
///
/// It must be guaranteed that the BSP will not access local data before this function is called,
/// otherwise dereference of non-Pod is undefined behavior.
pub unsafe fn bsp_init() {
    let start_base_va = __cpu_local_start as usize;
    let end_base_va = __cpu_local_end as usize;
    let nbytes = (end_base_va - start_base_va).align_up(PAGE_SIZE);
    extern "C" {
        fn __bsp_local_start();
    }
    // Safety: Both [__cpu_local_start, __cpu_local_end) and [__bsp_local_start, __bsp_local_end)
    // are reserved areas used for storing cpulocal data.
    unsafe {
        core::ptr::copy(
            start_base_va as *const u8,
            __bsp_local_start as usize as *mut u8,
            nbytes,
        );
    }
    // Safety: FS register is not used for any other purpose.
    unsafe {
        set_cpu_local_base_addr(__bsp_local_start as usize as u64);
    }
    prepare_cpu_local_data(0);
}

pub fn ap_init(cpu_id: u32) {
    let start_base_va = __cpu_local_start as usize;
    let end_base_va = __cpu_local_end as usize;
    let nbytes = (end_base_va - start_base_va).align_up(PAGE_SIZE);

    let local_segment = VmAllocOptions::new(nbytes / PAGE_SIZE)
        .is_contiguous(true)
        .need_dealloc(false)
        .alloc_contiguous()
        .unwrap();
    // Safety: [__cpu_local_start, __cpu_local_end) is reserved area used for storing cpulocal data.
    local_segment
        .writer()
        .write(&mut unsafe { slice::from_raw_parts(start_base_va as *const u8, nbytes) }.into());
    // Safety: `local_segment` is initialized to have the `need_dealloc` attribute, so this
    // memory will be dedicated to storing cpu local data.
    unsafe {
        set_cpu_local_base_addr(paddr_to_vaddr(local_segment.start_paddr()) as u64);
    }
    prepare_cpu_local_data(cpu_id);
}
