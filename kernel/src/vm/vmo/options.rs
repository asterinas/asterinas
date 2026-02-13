// SPDX-License-Identifier: MPL-2.0

//! Options for allocating root and child VMOs.

use core::sync::atomic::AtomicUsize;

use align_ext::AlignExt;
use ostd::mm::{FrameAllocOptions, Segment};
use xarray::XArray;

use super::{Vmo, VmoFlags, WritableMappingStatus};
use crate::{
    fs::utils::{CachePage, CachePageMeta, PageCacheBackend},
    prelude::*,
};

/// Options for allocating a root VMO.
///
/// # Examples
///
/// Creating a VMO:
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions};
///
/// let vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// ```
///
/// Creating a resizable VMO backed by 10 memory pages that may not be
/// physically contiguous:
///
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions, VmoFlags};
///
/// let vmo = VmoOptions::new(10 * PAGE_SIZE)
///     .flags(VmoFlags::RESIZABLE)
///     .alloc()
///     .unwrap();
/// ```
pub struct VmoOptions {
    size: usize,
    flags: VmoFlags,
    backend: Option<Weak<dyn PageCacheBackend>>,
}

impl VmoOptions {
    /// Creates a default set of options with the specified size of the VMO
    /// (in bytes).
    ///
    /// The size of the VMO will be rounded up to align with the page size.
    pub fn new(size: usize) -> Self {
        Self {
            size,
            flags: VmoFlags::empty(),
            backend: None,
        }
    }

    /// Sets the VMO flags.
    ///
    /// The default value is `VmoFlags::empty()`.
    ///
    /// For more information about the flags, see `VmoFlags`.
    pub fn flags(mut self, flags: VmoFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Sets the backend for the VMO.
    pub fn backend(mut self, backend: Weak<dyn PageCacheBackend>) -> Self {
        self.backend = Some(backend);
        self
    }
}

impl VmoOptions {
    /// Allocates the VMO according to the specified options.
    pub fn alloc(self) -> Result<Arc<Vmo>> {
        let VmoOptions {
            size,
            flags,
            backend,
            ..
        } = self;
        let vmo = alloc_vmo(size, flags, backend)?;
        Ok(Arc::new(vmo))
    }
}

fn alloc_vmo(
    size: usize,
    flags: VmoFlags,
    backend: Option<Weak<dyn PageCacheBackend>>,
) -> Result<Vmo> {
    let size = size.align_up(PAGE_SIZE);
    let pages = committed_pages_if_continuous(flags, size)?;
    let writable_mapping_status = WritableMappingStatus::default();
    Ok(Vmo {
        backend,
        flags,
        pages,
        size: AtomicUsize::new(size),
        writable_mapping_status,
    })
}

fn committed_pages_if_continuous(flags: VmoFlags, size: usize) -> Result<XArray<CachePage>> {
    if flags.contains(VmoFlags::CONTIGUOUS) {
        // if the vmo is continuous, we need to allocate frames for the vmo
        let frames_num = size / PAGE_SIZE;
        let segment: Segment<CachePageMeta> = FrameAllocOptions::new()
            .alloc_segment_with(frames_num, |_| CachePageMeta::default())?;
        let committed_pages = XArray::new();
        let mut locked_pages = committed_pages.lock();
        let mut cursor = locked_pages.cursor_mut(0);
        for frame in segment {
            cursor.store(frame);
            cursor.next();
        }
        drop(locked_pages);
        Ok(committed_pages)
    } else {
        // otherwise, we wait for the page is read or write
        Ok(XArray::new())
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::VmIo, prelude::*};

    use super::*;

    #[ktest]
    fn alloc_vmo() {
        let vmo = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
        assert_eq!(vmo.size(), PAGE_SIZE);
        // the vmo is zeroed once allocated
        assert_eq!(vmo.read_val::<usize>(0).unwrap(), 0);
    }

    #[ktest]
    fn alloc_continuous_vmo() {
        let vmo = VmoOptions::new(10 * PAGE_SIZE)
            .flags(VmoFlags::CONTIGUOUS)
            .alloc()
            .unwrap();
        assert_eq!(vmo.size(), 10 * PAGE_SIZE);
    }

    #[ktest]
    fn write_and_read() {
        let vmo = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
        let val = 42u8;
        // write val
        vmo.write_val(111, &val).unwrap();
        let read_val: u8 = vmo.read_val(111).unwrap();
        assert_eq!(val, read_val);
        // bit endian
        vmo.write_bytes(222, &[0x12, 0x34, 0x56, 0x78]).unwrap();
        let read_val: u32 = vmo.read_val(222).unwrap();
        assert_eq!(read_val, 0x78563412)
    }

    #[ktest]
    fn resize() {
        use crate::fs::utils::PageCacheOps;

        let vmo = VmoOptions::new(PAGE_SIZE)
            .flags(VmoFlags::RESIZABLE)
            .alloc()
            .unwrap();
        vmo.write_val(10, &42u8).unwrap();
        vmo.resize(2 * PAGE_SIZE, vmo.size()).unwrap();
        assert_eq!(vmo.size(), 2 * PAGE_SIZE);
        assert_eq!(vmo.read_val::<u8>(10).unwrap(), 42);
        vmo.write_val(PAGE_SIZE + 20, &123u8).unwrap();
        vmo.resize(PAGE_SIZE, vmo.size()).unwrap();
        assert_eq!(vmo.read_val::<u8>(10).unwrap(), 42);
    }
}
