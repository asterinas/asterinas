// SPDX-License-Identifier: MPL-2.0

use crate::allocator::CommonSizeClass;
use core::ops::Deref;
use ostd::{
    cpu::{
        local::{CpuLocalAllocator, DynamicCpuLocal},
        CpuId,
    },
    prelude::*,
    Error,
};

/// A wrapper over [`ostd::DynamicCpuLocal<T>`] to deallocate CPU-local objects on drop automatically.
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

/// Global allocators for dynamically-allocated CPU-Local objects.
static ALLOCATOR_8: CpuLocalAllocator<8> = CpuLocalAllocator::new();
static ALLOCATOR_16: CpuLocalAllocator<16> = CpuLocalAllocator::new();
static ALLOCATOR_32: CpuLocalAllocator<32> = CpuLocalAllocator::new();

/// Allocates a dynamically-allocated CPU-local object of type `T` and
/// initializes it with `init_values`.
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
