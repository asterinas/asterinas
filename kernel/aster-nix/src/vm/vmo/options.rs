// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

//! Options for allocating root and child VMOs.

use align_ext::AlignExt;
use aster_rights::{Rights, TRightSet, TRights};
use ostd::{
    collections::xarray::XArray,
    mm::{Frame, FrameAllocOptions},
};

use super::{Pager, Pages, Vmo, VmoFlags};
use crate::{prelude::*, vm::vmo::Vmo_};

/// Options for allocating a root VMO.
///
/// # Examples
///
/// Creating a VMO as a _dynamic_ capability with full access rights:
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions};
///
/// let vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// ```
///
/// Creating a VMO as a _static_ capability with all access rights:
/// ```
/// use aster_nix::prelude::*;
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions};
///
/// let vmo = VmoOptions::<Full>::new(PAGE_SIZE)
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
pub struct VmoOptions<R = Rights> {
    size: usize,
    flags: VmoFlags,
    rights: Option<R>,
    pager: Option<Arc<dyn Pager>>,
}

impl<R> VmoOptions<R> {
    /// Creates a default set of options with the specified size of the VMO
    /// (in bytes).
    ///
    /// The size of the VMO will be rounded up to align with the page size.
    pub fn new(size: usize) -> Self {
        Self {
            size,
            flags: VmoFlags::empty(),
            rights: None,
            pager: None,
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

    /// Sets the pager of the VMO.
    pub fn pager(mut self, pager: Arc<dyn Pager>) -> Self {
        self.pager = Some(pager);
        self
    }
}

impl VmoOptions<Rights> {
    /// Allocates the VMO according to the specified options.
    ///
    /// # Access rights
    ///
    /// The VMO is initially assigned full access rights.
    pub fn alloc(self) -> Result<Vmo<Rights>> {
        let VmoOptions {
            size, flags, pager, ..
        } = self;
        let vmo_ = alloc_vmo_(size, flags, pager)?;
        Ok(Vmo(Arc::new(vmo_), Rights::all()))
    }
}

impl<R: TRights> VmoOptions<TRightSet<R>> {
    /// Allocates the VMO according to the specified options.
    ///
    /// # Access rights
    ///
    /// The VMO is initially assigned the access rights represented
    /// by `R: TRights`.
    pub fn alloc(self) -> Result<Vmo<TRightSet<R>>> {
        let VmoOptions {
            size,
            flags,
            rights,
            pager,
        } = self;
        let vmo_ = alloc_vmo_(size, flags, pager)?;
        Ok(Vmo(Arc::new(vmo_), TRightSet(R::new())))
    }
}

fn alloc_vmo_(size: usize, flags: VmoFlags, pager: Option<Arc<dyn Pager>>) -> Result<Vmo_> {
    let size = size.align_up(PAGE_SIZE);
    let pages = {
        let pages = committed_pages_if_continuous(flags, size)?;
        if flags.contains(VmoFlags::RESIZABLE) {
            Pages::Resizable(Mutex::new((pages, size)))
        } else {
            Pages::Nonresizable(Mutex::new(pages), size)
        }
    };
    Ok(Vmo_ {
        pager,
        flags,
        pages,
    })
}

fn committed_pages_if_continuous(flags: VmoFlags, size: usize) -> Result<XArray<Frame>> {
    if flags.contains(VmoFlags::CONTIGUOUS) {
        // if the vmo is continuous, we need to allocate frames for the vmo
        let frames_num = size / PAGE_SIZE;
        let frames = FrameAllocOptions::new(frames_num)
            .is_contiguous(true)
            .alloc()?;
        let mut committed_pages = XArray::new();
        let mut cursor = committed_pages.cursor_mut(0);
        for frame in frames {
            cursor.store(frame);
            cursor.next();
        }
        drop(cursor);
        Ok(committed_pages)
    } else {
        // otherwise, we wait for the page is read or write
        Ok(XArray::new())
    }
}

#[cfg(ktest)]
mod test {
    use aster_rights::Full;
    use ostd::{mm::VmIo, prelude::*};

    use super::*;

    #[ktest]
    fn alloc_vmo() {
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap();
        assert_eq!(vmo.size(), PAGE_SIZE);
        // the vmo is zeroed once allocated
        assert_eq!(vmo.read_val::<usize>(0).unwrap(), 0);
    }

    #[ktest]
    fn alloc_continuous_vmo() {
        let vmo = VmoOptions::<Full>::new(10 * PAGE_SIZE)
            .flags(VmoFlags::CONTIGUOUS)
            .alloc()
            .unwrap();
        assert_eq!(vmo.size(), 10 * PAGE_SIZE);
    }

    #[ktest]
    fn write_and_read() {
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap();
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
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE)
            .flags(VmoFlags::RESIZABLE)
            .alloc()
            .unwrap();
        vmo.write_val(10, &42u8).unwrap();
        vmo.resize(2 * PAGE_SIZE).unwrap();
        assert_eq!(vmo.size(), 2 * PAGE_SIZE);
        assert_eq!(vmo.read_val::<u8>(10).unwrap(), 42);
        vmo.write_val(PAGE_SIZE + 20, &123u8).unwrap();
        vmo.resize(PAGE_SIZE).unwrap();
        assert_eq!(vmo.read_val::<u8>(10).unwrap(), 42);
    }
}
