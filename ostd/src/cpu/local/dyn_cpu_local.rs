// SPDX-License-Identifier: MPL-2.0

//! Dynamically-allocated CPU-local objects.

use alloc::vec::Vec;
use core::{marker::PhantomData, mem::ManuallyDrop};

use align_ext::AlignExt;
use bitvec::prelude::{bitvec, BitVec};

use super::{AnyStorage, CpuLocal};
use crate::{
    cpu::{all_cpus, num_cpus, CpuId, PinCurrentCpu},
    mm::{paddr_to_vaddr, FrameAllocOptions, Segment, Vaddr, PAGE_SIZE},
    sync::SpinLock,
    trap::DisabledLocalIrqGuard,
    Result,
};

/// A dynamically-allocated storage for a CPU-local variable of type `T`.
///
/// Such a CPU-local storage is not intended to be allocated directly.
/// Use [`CpuLocalAllocator`] instead.
pub struct DynamicStorage<T>(*const T);

unsafe impl<T> AnyStorage<T> for DynamicStorage<T> {
    fn get_ptr_on_current(&self, guard: &DisabledLocalIrqGuard) -> *const T {
        self.get_ptr_on_target(guard.current_cpu())
    }

    fn get_ptr_on_target(&self, cpu_id: CpuId) -> *const T {
        let bsp_va = self.0 as usize;
        let va = bsp_va + cpu_id.as_usize() * CHUNK_SIZE;
        va as *const T
    }
}

impl<T> Drop for DynamicStorage<T> {
    fn drop(&mut self) {
        panic!(
            "Do not drop `CpuLocal<T, DynamicStorage<T>>` directly. \
            Use `CpuLocalAllocator::dealloc<T>` instead."
        );
    }
}

impl<T> CpuLocal<T, DynamicStorage<T>> {
    /// Creates a new dynamically-allocated CPU-local object, and
    /// initializes it with `init_values`.
    ///
    /// The given `ptr` points to the variable located on the BSP.
    /// The corresponding variables on all CPUs are initialized to zero.
    ///
    /// Please do not call this function directly. Instead, use
    /// [`CpuLocalAllocator`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that the new per-CPU object belongs to an
    /// existing [`DynCpuLocalChunk`], and does not overlap with any existing
    /// CPU-local object.
    unsafe fn __new_dynamic(ptr: *const T, init_values: &mut impl FnMut(CpuId) -> T) -> Self {
        let storage = DynamicStorage(ptr);
        for cpu in all_cpus() {
            let ptr = storage.get_ptr_on_target(cpu);
            // SAFETY: `ptr` points to the local variable of `storage` on `cpu`.
            unsafe {
                let mut_ptr = ptr as *mut T;
                *mut_ptr = init_values(cpu);
            }
        }

        Self {
            storage,
            phantom: PhantomData,
        }
    }
}

const CHUNK_SIZE: usize = PAGE_SIZE;

/// Manages dynamically-allocated CPU-local chunks.
///
/// Each CPU owns a chunk of size `CHUNK_SIZE`, and the chunks are laid
/// out contiguously in the order of CPU IDs. Per-CPU variables lie within
/// the chunks.
struct DynCpuLocalChunk<const ITEM_SIZE: usize> {
    segment: Segment<()>,
    bitmap: BitVec,
}

impl<const ITEM_SIZE: usize> DynCpuLocalChunk<ITEM_SIZE> {
    /// Creates a new dynamically-allocated CPU-local chunk.
    fn new() -> Result<Self> {
        let total_chunk_size = (CHUNK_SIZE * num_cpus()).align_up(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(total_chunk_size / PAGE_SIZE)?;

        let num_items = CHUNK_SIZE / ITEM_SIZE;
        debug_assert!(num_items * ITEM_SIZE == CHUNK_SIZE);
        Ok(Self {
            segment,
            bitmap: bitvec![0; num_items],
        })
    }

    /// Returns a pointer to the local chunk owned by the BSP.
    fn get_start_vaddr(&self) -> Vaddr {
        paddr_to_vaddr(self.segment.start_paddr())
    }

    /// Allocates a CPU-local object from the chunk, and
    /// initializes it with `init_values`.
    ///
    /// If the chunk is full, returns None.
    fn alloc<T>(
        &mut self,
        init_values: &mut impl FnMut(CpuId) -> T,
    ) -> Option<CpuLocal<T, DynamicStorage<T>>> {
        debug_assert!(core::mem::size_of::<T>() <= ITEM_SIZE);
        let index = self.bitmap.first_zero()?;
        self.bitmap.set(index, true);
        // SAFETY: `index` refers to an available position in the chunk
        // for allocating a new CPU-local object.
        unsafe {
            let vaddr = self.get_start_vaddr() + index * ITEM_SIZE;
            Some(CpuLocal::__new_dynamic(vaddr as *const T, init_values))
        }
    }

    /// Gets the index of a dynamically-allocated CPU-Local object
    /// within the chunk. If the object does not belong to the chunk,
    /// returns `None`.
    fn get_item_index<T>(&mut self, cpu_local: &CpuLocal<T, DynamicStorage<T>>) -> Option<usize> {
        let vaddr = cpu_local.storage.0 as Vaddr;
        let start_vaddr = self.get_start_vaddr();
        let offset = vaddr.checked_sub(start_vaddr)?;
        if offset >= CHUNK_SIZE || offset % ITEM_SIZE != 0 {
            None
        } else {
            Some(offset / ITEM_SIZE)
        }
    }

    /// Deallocates a previously allocated CPU-local object.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `cpu_local` belongs to this chunk,
    /// and `index` is the correct index corresponding to `cpu_local`
    /// within this chunk.
    unsafe fn dealloc<T>(&mut self, index: usize, cpu_local: CpuLocal<T, DynamicStorage<T>>) {
        debug_assert!(index == self.get_item_index(&cpu_local).unwrap());
        self.bitmap.set(index, false);
        let _ = ManuallyDrop::new(cpu_local);
    }

    /// Checks whether the chunk is full.
    fn is_full(&self) -> bool {
        self.bitmap.all()
    }

    /// Checks whether the chunk is empty.
    fn is_empty(&self) -> bool {
        self.bitmap.not_any()
    }
}

/// Allocator for dynamically-allocated CPU-Local objects.
pub struct CpuLocalAllocator<const ITEM_SIZE: usize> {
    chunks: SpinLock<Vec<DynCpuLocalChunk<ITEM_SIZE>>>,
}

impl<const ITEM_SIZE: usize> CpuLocalAllocator<ITEM_SIZE> {
    /// Creates a new allocator for dynamically-allocated CPU-local objects.
    #[expect(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            chunks: SpinLock::new(Vec::new()),
        }
    }

    /// Allocates a CPU-local object and initializes it with `init_values`.
    pub fn alloc<T>(
        &'static self,
        init_values: &mut impl FnMut(CpuId) -> T,
    ) -> Result<CpuLocal<T, DynamicStorage<T>>> {
        debug_assert!(core::mem::size_of::<T>() <= ITEM_SIZE);
        let mut chunks = self.chunks.lock();
        for chunk in chunks.iter_mut() {
            if !chunk.is_full() {
                let cpu_local = chunk.alloc::<T>(init_values).unwrap();
                return Ok(cpu_local);
            }
        }
        let mut new_chunk = DynCpuLocalChunk::<ITEM_SIZE>::new()?;
        let cpu_local = new_chunk.alloc::<T>(init_values).unwrap();
        chunks.push(new_chunk);
        Ok(cpu_local)
    }

    /// Deallocates a CPU-local object.
    pub fn dealloc<T>(&self, cpu_local: CpuLocal<T, DynamicStorage<T>>) {
        let mut chunks = self.chunks.lock();

        let mut chunk_index = None;
        for (i, chunk) in chunks.iter_mut().enumerate() {
            if let Some(index) = chunk.get_item_index(&cpu_local) {
                // SAFETY: The safety is ensured by `get_item_index`.
                unsafe {
                    chunk.dealloc(index, cpu_local);
                }
                chunk_index = Some(i);
                break;
            }
        }
        let chunk_index = chunk_index.unwrap();
        if chunks[chunk_index].is_empty() && chunks.iter().filter(|c| c.is_empty()).count() > 1 {
            chunks.remove(chunk_index);
        }
    }
}
