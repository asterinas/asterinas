// SPDX-License-Identifier: MPL-2.0

//! Dynamically-allocated CPU-local objects.

use core::{marker::PhantomData, mem::ManuallyDrop, ptr::NonNull};

use bitvec::prelude::{bitvec, BitVec};

use super::{AnyStorage, CpuLocal};
use crate::{
    cpu::{all_cpus, num_cpus, CpuId, PinCurrentCpu},
    mm::{paddr_to_vaddr, FrameAllocOptions, Segment, Vaddr, PAGE_SIZE},
    trap::irq::DisabledLocalIrqGuard,
    Result,
};

/// A dynamically-allocated storage for a CPU-local variable of type `T`.
///
/// Such a CPU-local storage should be allocated and deallocated by
/// [`DynCpuLocalChunk`], not directly. Dropping it without deallocation
/// will cause panic.
///
/// When dropping a `CpuLocal<T, DynamicStorage<T>>`, we have no way to know
/// which `DynCpuLocalChunk` the CPU-local object was originally allocated
/// from. Therefore, we rely on the user to correctly manage the corresponding
/// `DynCpuLocalChunk`, ensuring that both allocation and deallocation of
/// `CpuLocal<T, DynamicStorage<T>>` occur within the same chunk.
///
/// To properly deallocate the CPU-local object, the user must explicitly call
/// the appropriate `DynCpuLocalChunk`'s `try_dealloc<T>()`. Otherwise,
/// dropping it directly will cause a panic.
pub struct DynamicStorage<T>(NonNull<T>);

unsafe impl<T> AnyStorage<T> for DynamicStorage<T> {
    fn get_ptr_on_current(&self, guard: &DisabledLocalIrqGuard) -> *const T {
        self.get_ptr_on_target(guard.current_cpu())
    }

    fn get_ptr_on_target(&self, cpu_id: CpuId) -> *const T {
        let bsp_va = self.0.as_ptr() as usize;
        let va = bsp_va + cpu_id.as_usize() * CHUNK_SIZE;
        va as *mut T
    }

    fn get_mut_ptr_on_target(&mut self, cpu: CpuId) -> *mut T {
        self.get_ptr_on_target(cpu).cast_mut()
    }
}

impl<T> Drop for DynamicStorage<T> {
    fn drop(&mut self) {
        panic!(
            "Do not drop `DynamicStorage<T>` directly. \
            Use `DynCpuLocalChunk::try_dealloc<T>` instead."
        );
    }
}

impl<T: Sync + alloc::fmt::Debug + 'static> alloc::fmt::Debug for CpuLocal<T, DynamicStorage<T>> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut list = f.debug_list();
        for cpu in all_cpus() {
            let val = self.get_on_cpu(cpu);
            list.entry(&(&cpu, val));
        }
        list.finish()
    }
}

impl<T> CpuLocal<T, DynamicStorage<T>> {
    /// Creates a new dynamically-allocated CPU-local object, and
    /// initializes it with `init_values`.
    ///
    /// The given `ptr` points to the variable located on the BSP.
    ///
    /// Please do not call this function directly. Instead, use
    /// [`DynCpuLocalChunk::alloc`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that
    ///  - the new per-CPU object belongs to an existing
    ///    [`DynCpuLocalChunk`], and does not overlap with any
    ///    existing CPU-local object;
    ///  - the `ITEM_SIZE` of the [`DynCpuLocalChunk`] satisfies
    ///    the layout requirement of `T`.
    unsafe fn __new_dynamic(ptr: *mut T, init_values: &mut impl FnMut(CpuId) -> T) -> Self {
        let mut storage = DynamicStorage(NonNull::new(ptr).unwrap());
        for cpu in all_cpus() {
            let ptr = storage.get_mut_ptr_on_target(cpu);
            // SAFETY:
            //  - `ptr` is valid for writes, because:
            //    - The `DynCpuLocalChunk` slot is non-null and dereferenceable.
            //    - This initialization occurs before any other code can access
            //      the memory. References to the data may only be created
            //      after `Self` is created, ensuring exclusive access by the
            //      current task.
            //  - `ptr` is properly aligned, as the caller guarantees that the
            //    layout requirement is satisfied.
            unsafe {
                core::ptr::write(ptr, init_values(cpu));
            }
        }

        Self {
            storage,
            phantom: PhantomData,
        }
    }
}

const CHUNK_SIZE: usize = PAGE_SIZE;

/// Footer metadata to describe a `SSTable`.
#[derive(Debug, Clone, Copy)]
struct DynCpuLocalMeta;
crate::impl_frame_meta_for!(DynCpuLocalMeta);

/// Manages dynamically-allocated CPU-local chunks.
///
/// Each CPU owns a chunk of size `CHUNK_SIZE`, and the chunks are laid
/// out contiguously in the order of CPU IDs. Per-CPU variables lie within
/// the chunks.
pub struct DynCpuLocalChunk<const ITEM_SIZE: usize> {
    segment: ManuallyDrop<Segment<DynCpuLocalMeta>>,
    bitmap: BitVec,
}

impl<const ITEM_SIZE: usize> DynCpuLocalChunk<ITEM_SIZE> {
    /// Creates a new dynamically-allocated CPU-local chunk.
    pub fn new() -> Result<Self> {
        let total_chunk_size = CHUNK_SIZE * num_cpus();
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(total_chunk_size.div_ceil(PAGE_SIZE), |_| DynCpuLocalMeta)?;

        let num_items = CHUNK_SIZE / ITEM_SIZE;
        const { assert!(CHUNK_SIZE % ITEM_SIZE == 0) };

        Ok(Self {
            segment: ManuallyDrop::new(segment),
            bitmap: bitvec![0; num_items],
        })
    }

    /// Returns a pointer to the local chunk owned by the BSP.
    fn start_vaddr(&self) -> Vaddr {
        paddr_to_vaddr(self.segment.start_paddr())
    }

    /// Allocates a CPU-local object from the chunk, and
    /// initializes it with `init_values`.
    ///
    /// Returns `None` if the chunk is full.
    pub fn alloc<T>(
        &mut self,
        init_values: &mut impl FnMut(CpuId) -> T,
    ) -> Option<CpuLocal<T, DynamicStorage<T>>> {
        const {
            assert!(ITEM_SIZE.is_power_of_two());
            assert!(core::mem::size_of::<T>() <= ITEM_SIZE);
            assert!(core::mem::align_of::<T>() <= ITEM_SIZE);
        }

        let index = self.bitmap.first_zero()?;
        self.bitmap.set(index, true);
        // SAFETY:
        //  - `index` refers to an available position in the chunk
        //    for allocating a new CPU-local object.
        //  - We have checked the size and alignment requirement
        //    for `T` above.
        unsafe {
            let vaddr = self.start_vaddr() + index * ITEM_SIZE;
            Some(CpuLocal::__new_dynamic(vaddr as *mut T, init_values))
        }
    }

    /// Gets the index of a dynamically-allocated CPU-local object
    /// within the chunk.
    ///
    /// Returns `None` if the object does not belong to the chunk.
    fn get_item_index<T>(&mut self, cpu_local: &CpuLocal<T, DynamicStorage<T>>) -> Option<usize> {
        let vaddr = cpu_local.storage.0.as_ptr() as Vaddr;
        let start_vaddr = self.start_vaddr();

        let offset = vaddr.checked_sub(start_vaddr)?;
        if offset > CHUNK_SIZE {
            return None;
        }

        debug_assert_eq!(offset % ITEM_SIZE, 0);

        Some(offset / ITEM_SIZE)
    }

    /// Attempts to deallocate a previously allocated CPU-local object.
    ///
    /// Returns `Err(cpu_local)` if the object does not belong to this chunk.
    pub fn try_dealloc<T>(
        &mut self,
        mut cpu_local: CpuLocal<T, DynamicStorage<T>>,
    ) -> core::result::Result<(), CpuLocal<T, DynamicStorage<T>>> {
        let Some(index) = self.get_item_index(&cpu_local) else {
            return Err(cpu_local);
        };

        self.bitmap.set(index, false);
        for cpu in all_cpus() {
            let ptr = cpu_local.storage.get_mut_ptr_on_target(cpu);
            // SAFETY:
            //  - `ptr` is valid for both reads and writes, because:
            //    - The pointer of the CPU-local object on `cpu` is
            //      non-null and dereferenceable.
            //    - We can mutably borrow the CPU-local object on `cpu`
            //      because we have the exclusive access to `cpu_local`.
            //  - The pointer of the CPU-local object is properly aligned.
            //  - The pointer of the CPU-local object points to a valid
            //    instance of `T`.
            //  - After the deallocation, no one will access the
            //    dropped CPU-local object, since we explicitly forget
            //    the `cpu_local`.
            unsafe {
                core::ptr::drop_in_place(ptr);
            }
        }
        let _ = ManuallyDrop::new(cpu_local);
        Ok(())
    }

    /// Checks whether the chunk is full.
    pub fn is_full(&self) -> bool {
        self.bitmap.all()
    }

    /// Checks whether the chunk is empty.
    pub fn is_empty(&self) -> bool {
        self.bitmap.not_any()
    }
}

impl<const ITEM_SIZE: usize> Drop for DynCpuLocalChunk<ITEM_SIZE> {
    fn drop(&mut self) {
        if self.is_empty() {
            // SAFETY: The `segment` does not contain any CPU-local objects.
            // It is the last time the `segment` is accessed, and it will be
            // dropped only once.
            unsafe { ManuallyDrop::drop(&mut self.segment) }
        } else {
            // Leak the `segment` and panic.
            panic!("Dropping `DynCpuLocalChunk` while some CPU-local objects are still alive");
        }
    }
}
