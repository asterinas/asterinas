// SPDX-License-Identifier: MPL-2.0

//! CPU.

use alloc::{slice, vec::Vec};
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

use align_ext::AlignExt;
use bitvec::{order::Lsb0, slice::IterOnes, vec::BitVec};

use crate::{
    smp::CPUNUM,
    task::{disable_preempt, DisablePreemptGuard},
    vm::{paddr_to_vaddr, Vaddr, VmAllocOptions, VmIo, PAGE_SIZE},
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
///
/// The CPU ID is strategically placed at the beginning of the CPU local storage area to facilitate
/// fast access, allowing the ID to be retrieved with a single memory access.
pub fn this_cpu() -> u32 {
    // Safety: the cpu ID is stored at the beginning of the cpu local area.
    unsafe { core::ptr::read_volatile(get_cpu_local_base_addr() as usize as *mut u32) }
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
///     static FOO: AtomicBool = AtomicBool::new(false);
///
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

#[macro_export]
macro_rules! lazy_cpu_local {
    () => {};
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        lazy_cpu_local!(@MAKE TY, $(#[$attr])*, $vis, $name);
        lazy_cpu_local!(@TAIL, $name :$t = $init );
        $crate::lazy_cpu_local!($($rest)*);
    };

    (@TAIL, $name:ident : $t:ty = $init:expr) => {
        impl $name {
            pub fn borrow(&self) -> $crate::cpu::CpuLocalGuard<$t>{
                #[inline(always)]
                fn __static_ref_initialize() -> $t { $init }

                #[inline(always)]
                fn __stability() -> $crate::cpu::CpuLocalGuard<'static, $t> {
                    #[link_section = ".cpu_local"]
                    static LAZY: $crate::cpu::CpuLocal<spin::Once<$t>> = unsafe { $crate::cpu::CpuLocal::new(spin::Once::new()) };
                    let guard = LAZY.borrow();
                    guard.call_once(__static_ref_initialize);
                    let data_ref = unsafe{ LAZY.borrow_unchecked().get_mut().unwrap() };
                    $crate::cpu::CpuLocalGuard::new(core::cell::UnsafeCell::from_mut(data_ref), $crate::task::disable_preempt())
                }
                __stability()
            }
        }
    };

    (@MAKE TY, $(#[$attr:meta])*, $vis:vis, $name:ident) => {
        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        $(#[$attr])*
        $vis struct $name {__private_field: ()}
        #[doc(hidden)]
        $vis static $name: $name = $name {__private_field: ()};
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
pub struct CpuLocal<T: Unpin>(UnsafeCell<T>);

// Safety. At any given time, only one task can access the inner value T of a cpu-local variable.
unsafe impl<T: Unpin> Sync for CpuLocal<T> {}

impl<T: Unpin> CpuLocal<T> {
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

    pub fn borrow(&self) -> CpuLocalGuard<T> {
        let guard = disable_preempt();
        CpuLocalGuard {
            data: self.data(),
            guard,
        }
    }

    /// Borrows the CPU-local data without providing a guard for preemption safety.
    ///
    /// # Safety
    ///
    /// The caller must ensure that preemption does not lead to data races.
    pub unsafe fn borrow_unchecked(&self) -> &'static mut T {
        &mut *self.data().get()
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

pub struct CpuLocalGuard<'a, T: Unpin> {
    data: &'a UnsafeCell<T>,
    guard: DisablePreemptGuard,
}

impl<'a, T: Unpin> CpuLocalGuard<'a, T> {
    pub fn new(data: &'a UnsafeCell<T>, guard: DisablePreemptGuard) -> Self {
        CpuLocalGuard { data, guard }
    }

    pub fn inner_data(&self) -> &UnsafeCell<T> {
        self.data
    }
}

impl<'a, T: Unpin> Deref for CpuLocalGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // Safety: Now that the local IRQs are disabled, this CPU-local object can only be
        // accessed by the current task/thread.
        unsafe { &*self.data.get() }
    }
}

impl<'a, T: Unpin> DerefMut for CpuLocalGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: Now that the local IRQs are disabled, this CPU-local object can only be
        // accessed by the current task/thread.
        unsafe { &mut *self.data.get() }
    }
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
    // are reserved areas used for storing cpu local data.
    unsafe {
        core::ptr::copy(
            start_base_va as *const u8,
            __bsp_local_start as usize as *mut u8,
            nbytes,
        );
        core::ptr::write_volatile(__bsp_local_start as usize as *mut u32, 0_u32);
    }
    // Safety: FS register is not used for any other purpose.
    unsafe {
        set_cpu_local_base_addr(__bsp_local_start as u64);
    }
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
    local_segment.write_val(0, &cpu_id).unwrap();
    // Safety: `local_segment` is initialized to have the `need_dealloc` attribute, so this
    // memory will be dedicated to storing cpu local data.
    unsafe {
        set_cpu_local_base_addr(paddr_to_vaddr(local_segment.start_paddr()) as u64);
    }
}
