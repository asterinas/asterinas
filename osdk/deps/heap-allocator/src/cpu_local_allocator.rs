// SPDX-License-Identifier: MPL-2.0

use crate::allocator::CommonSizeClass;
use alloc::vec::Vec;
use core::ops::Deref;
use ostd::{
    cpu::{
        local::{DynCpuLocalChunk, DynamicCpuLocal},
        CpuId,
    },
    prelude::*,
    sync::SpinLock,
    Error,
};

/// Allocator for dynamically-allocated CPU-local objects.
struct CpuLocalAllocator<const ITEM_SIZE: usize> {
    chunks: SpinLock<Vec<DynCpuLocalChunk<ITEM_SIZE>>>,
}

impl<const ITEM_SIZE: usize> CpuLocalAllocator<ITEM_SIZE> {
    /// Creates a new allocator for dynamically-allocated CPU-local objects.
    pub(self) const fn new() -> Self {
        Self {
            chunks: SpinLock::new(Vec::new()),
        }
    }

    /// Allocates a CPU-local object and initializes it with `init_values`.
    pub(self) fn alloc<T>(
        &'static self,
        init_values: &mut impl FnMut(CpuId) -> T,
    ) -> Result<DynamicCpuLocal<T>> {
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
    pub(self) fn dealloc<T>(&self, cpu_local: DynamicCpuLocal<T>) {
        let mut cpu_local = cpu_local;
        let mut chunks = self.chunks.lock();

        let mut chunk_index = None;
        for (i, chunk) in chunks.iter_mut().enumerate() {
            match chunk.try_dealloc(cpu_local) {
                Ok(()) => {
                    chunk_index = Some(i);
                    break;
                }
                Err(returned) => cpu_local = returned,
            }
        }
        let chunk_index = chunk_index.unwrap();
        if chunks[chunk_index].is_empty() && chunks.iter().filter(|c| c.is_empty()).count() > 1 {
            chunks.swap_remove(chunk_index);
        }
    }
}

/// A wrapper over [`DynamicCpuLocal<T>`] to deallocate CPU-local objects on
/// drop automatically.
pub struct CpuLocalBox<T>(Option<DynamicCpuLocal<T>>);

impl<T> Deref for CpuLocalBox<T> {
    type Target = DynamicCpuLocal<T>;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().unwrap()
    }
}

impl<T> Drop for CpuLocalBox<T> {
    fn drop(&mut self) {
        let cpu_local = self.0.take().unwrap();
        dealloc_cpu_local(cpu_local);
    }
}

/// Global allocators for dynamically-allocated CPU-local objects.
static ALLOCATOR_8: CpuLocalAllocator<8> = CpuLocalAllocator::new();
static ALLOCATOR_16: CpuLocalAllocator<16> = CpuLocalAllocator::new();
static ALLOCATOR_32: CpuLocalAllocator<32> = CpuLocalAllocator::new();

/// Allocates a dynamically-allocated CPU-local object of type `T` and
/// initializes it with `init_values`.
///
/// Currently, the size of `T` must be no larger than 32 bytes.
pub fn alloc_cpu_local<T>(mut init_values: impl FnMut(CpuId) -> T) -> Result<CpuLocalBox<T>> {
    let size = core::mem::size_of::<T>();
    let class = CommonSizeClass::from_size(size).ok_or(Error::InvalidArgs)?;
    let cpu_local = match class {
        CommonSizeClass::Bytes8 => ALLOCATOR_8.alloc::<T>(&mut init_values),
        CommonSizeClass::Bytes16 => ALLOCATOR_16.alloc::<T>(&mut init_values),
        CommonSizeClass::Bytes32 => ALLOCATOR_32.alloc::<T>(&mut init_values),
        // TODO: Support contiguous allocations for larger sizes.
        // Since cache lines are normally 64 bytes, when allocating CPU-local
        // objects with larger sizes, we should allocate a `Vec` with size
        // `num_cpus()` instead.
        _ => Err(Error::InvalidArgs),
    }?;
    Ok(CpuLocalBox(Some(cpu_local)))
}

/// Deallocates a dynamically-allocated CPU-local object of type `T`.
fn dealloc_cpu_local<T>(cpu_local: DynamicCpuLocal<T>) {
    let size = core::mem::size_of::<T>();
    let class = CommonSizeClass::from_size(size).unwrap();
    match class {
        CommonSizeClass::Bytes8 => ALLOCATOR_8.dealloc(cpu_local),
        CommonSizeClass::Bytes16 => ALLOCATOR_16.dealloc(cpu_local),
        CommonSizeClass::Bytes32 => ALLOCATOR_32.dealloc(cpu_local),
        _ => todo!(),
    }
}
