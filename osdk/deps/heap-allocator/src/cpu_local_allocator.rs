// SPDX-License-Identifier: MPL-2.0

use crate::allocator::CommonSizeClass;
use alloc::vec::Vec;
use core::{alloc::Layout, ops::Deref};
use ostd::{
    Error,
    cpu::{
        CpuId,
        local::{DynCpuLocalChunk, DynamicCpuLocal},
    },
    prelude::*,
    sync::SpinLock,
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
    let layout = Layout::from_size_align(size_of::<T>(), align_of::<T>()).unwrap();
    let class = CommonSizeClass::from_layout(layout).ok_or(Error::InvalidArgs)?;
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
    let layout = Layout::from_size_align(size_of::<T>(), align_of::<T>()).unwrap();
    let class = CommonSizeClass::from_layout(layout).unwrap();
    match class {
        CommonSizeClass::Bytes8 => ALLOCATOR_8.dealloc(cpu_local),
        CommonSizeClass::Bytes16 => ALLOCATOR_16.dealloc(cpu_local),
        CommonSizeClass::Bytes32 => ALLOCATOR_32.dealloc(cpu_local),
        _ => todo!(),
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use core::{cmp::PartialEq, fmt::Debug};
    use ostd::{cpu::CpuId, util::id_set::Id};

    #[derive(Debug, PartialEq)]
    #[repr(align(2))]
    struct Aligned2(u8);
    #[derive(Debug, PartialEq)]
    #[repr(align(4))]
    struct Aligned4(u8);
    #[derive(Debug, PartialEq)]
    #[repr(align(8))]
    struct Aligned8(u8);
    #[derive(Debug, PartialEq)]
    #[repr(align(16))]
    struct Aligned16(u8);
    #[derive(Debug, PartialEq)]
    #[repr(align(32))]
    struct Aligned32(u8);

    #[track_caller]
    fn alloc_dealloc_cpu_local<T: 'static + Debug + PartialEq + Sync>(init_values: fn(CpuId) -> T) {
        let values = ostd::cpu::all_cpus().map(init_values).collect::<Vec<_>>();
        let cpu_local = alloc_cpu_local(init_values).expect("Failed to allocate CPU-local object");
        for (cpu, value) in ostd::cpu::all_cpus().zip(values.iter()) {
            let allocated = cpu_local.get_on_cpu(cpu);
            assert_eq!(*allocated, *value);
        }
        // Dropping `cpu_local` should deallocate the object.
    }

    #[ktest]
    fn alloc_dealloc_cpu_local_various_types() {
        alloc_dealloc_cpu_local(|cpu| cpu.as_usize() as u8);
        alloc_dealloc_cpu_local(|cpu| cpu.as_usize() as u16);
        alloc_dealloc_cpu_local(|cpu| cpu.as_usize() as u32);
        alloc_dealloc_cpu_local(|cpu| cpu.as_usize() as u64);
        alloc_dealloc_cpu_local(|cpu| cpu.as_usize());
        alloc_dealloc_cpu_local(|cpu| Aligned2(cpu.as_usize() as u8));
        alloc_dealloc_cpu_local(|cpu| Aligned4(cpu.as_usize() as u8));
        alloc_dealloc_cpu_local(|cpu| Aligned8(cpu.as_usize() as u8));
        alloc_dealloc_cpu_local(|cpu| Aligned16(cpu.as_usize() as u8));
        alloc_dealloc_cpu_local(|cpu| Aligned32(cpu.as_usize() as u8));
    }
}
