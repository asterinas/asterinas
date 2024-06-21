// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

//! Options for allocating root and child VMOs.

use core::{marker::PhantomData, ops::Range};

use align_ext::AlignExt;
use aster_rights::{Dup, Rights, TRightSet, TRights, Write};
use aster_rights_proc::require;
use ostd::{
    collections::xarray::XArray,
    mm::{Frame, FrameAllocOptions},
};
use typeflags_util::{SetExtend, SetExtendOp};

use super::{Pager, Pages, Vmo, VmoFlags, VmoMark, VmoRightsOp};
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
            Pages::Nonresizable(Arc::new(Mutex::new(pages)), size)
        }
    };
    Ok(Vmo_ {
        pager,
        flags,
        page_idx_offset: 0,
        pages,
    })
}

fn committed_pages_if_continuous(flags: VmoFlags, size: usize) -> Result<XArray<Frame, VmoMark>> {
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

/// Options for allocating a child VMO out of a parent VMO.
///
/// # Examples
///
/// A child VMO created from a parent VMO of _dynamic_ capability is also a
/// _dynamic_ capability.
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions};
///
/// let parent_vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// let child_vmo = parent_vmo.new_slice_child(0..PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// assert!(parent_vmo.rights() == child_vmo.rights());
/// ```
///
/// A child VMO created from a parent VMO of _static_ capability is also a
/// _static_ capability.
/// ```
/// use aster_nix::prelude::*;
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
///
/// let parent_vmo: Vmo<Full> = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// let child_vmo: Vmo<Full> = parent_vmo.new_slice_child(0..PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// assert!(parent_vmo.rights() == child_vmo.rights());
/// ```
///
/// Normally, a child VMO is initially given the same set of access rights
/// as its parent (as shown above). But there is one exception:
/// if the child VMO is created as a COW child, then it is granted the Write
/// right regardless of whether the parent is writable or not.
///
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
///
/// let parent_vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap()
///     .restrict(Rights::DUP | Rights::READ);
/// let child_vmo = parent_vmo.new_cow_child(0..PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// assert!(child_vmo.rights().contains(Rights::WRITE));
/// ```
///
/// The above rule for COW VMO children also applies to static capabilities.
///
/// ```
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
///
/// let parent_vmo = VmoOptions::<TRights![Read, Dup]>::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// let child_vmo = parent_vmo.new_cow_child(0..PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// assert!(child_vmo.rights().contains(Rights::WRITE));
/// ```
///
/// One can set VMO flags for a child VMO. Currently, the only flag that is
/// valid when creating VMO children is `VmoFlags::RESIZABLE`.
/// Note that a slice VMO child and its parent cannot not be resizable.
///
/// ```rust
/// use aster_nix::vm::{PAGE_SIZE, VmoOptions};
///
/// let parent_vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// let child_vmo = parent_vmo.new_cow_child(0..PAGE_SIZE)
///     // Make the child resizable!
///     .flags(VmoFlags::RESIZABLE)
///     .alloc()
///     .unwrap();
/// assert!(parent_vmo.rights() == child_vmo.rights());
/// ```
pub struct VmoChildOptions<R, C> {
    parent: Vmo<R>,
    range: Range<usize>,
    flags: VmoFlags,
    // Specifies whether the child is a slice or a COW
    marker: PhantomData<C>,
}

impl<R: TRights> VmoChildOptions<TRightSet<R>, VmoSliceChild> {
    /// Creates a default set of options for creating a slice VMO child.
    ///
    /// A slice child of a VMO, which has direct access to a range of memory
    /// pages in the parent VMO. In other words, any updates of the parent will
    /// reflect on the child, and vice versa.
    ///
    /// The range of a child must be within that of the parent.
    #[require(R > Dup)]
    pub fn new_slice(parent: Vmo<TRightSet<R>>, range: Range<usize>) -> Self {
        Self {
            flags: parent.flags() & Self::PARENT_FLAGS_MASK,
            parent,
            range,
            marker: PhantomData,
        }
    }
}

impl VmoChildOptions<Rights, VmoSliceChild> {
    /// Creates a default set of options for creating a slice VMO child.
    ///
    /// User should ensure parent have dup rights, otherwise this function will panic
    ///
    /// A slice child of a VMO, which has direct access to a range of memory
    /// pages in the parent VMO. In other words, any updates of the parent will
    /// reflect on the child, and vice versa.
    ///
    /// The range of a child must be within that of the parent.
    pub fn new_slice_rights(parent: Vmo<Rights>, range: Range<usize>) -> Self {
        parent
            .check_rights(Rights::DUP)
            .expect("function new_slice_rights should called with rights Dup");
        Self {
            flags: parent.flags(),
            parent,
            range,
            marker: PhantomData,
        }
    }
}

impl<R> VmoChildOptions<R, VmoSliceChild> {
    /// Flags that a VMO child inherits from its parent.
    pub const PARENT_FLAGS_MASK: VmoFlags =
        VmoFlags::from_bits(VmoFlags::CONTIGUOUS.bits | VmoFlags::DMA.bits).unwrap();
    /// Flags that a VMO child may differ from its parent.
    pub const CHILD_FLAGS_MASK: VmoFlags = VmoFlags::empty();

    /// Sets the VMO flags.
    ///
    /// Only the flags among `Self::CHILD_FLAGS_MASK` may be set through this
    /// method.
    ///
    /// To set `VmoFlags::RESIZABLE`, the child must be COW.
    ///
    /// The default value is `VmoFlags::empty()`.
    pub fn flags(mut self, flags: VmoFlags) -> Self {
        let inherited_flags = self.flags & Self::PARENT_FLAGS_MASK;
        self.flags = inherited_flags | (flags & Self::CHILD_FLAGS_MASK);
        self
    }
}

impl<R> VmoChildOptions<R, VmoCowChild> {
    /// Flags that a VMO child inherits from its parent.
    pub const PARENT_FLAGS_MASK: VmoFlags =
        VmoFlags::from_bits(VmoFlags::CONTIGUOUS.bits | VmoFlags::DMA.bits).unwrap();
    /// Flags that a VMO child may differ from its parent.
    pub const CHILD_FLAGS_MASK: VmoFlags = VmoFlags::RESIZABLE;
    /// Creates a default set of options for creating a copy-on-write (COW)
    /// VMO child.
    ///
    /// A COW VMO child behaves as if all its
    /// memory pages are copied from the parent VMO upon creation, although
    /// the copying is done lazily when the parent's memory pages are updated.
    ///
    /// The range of a child may go beyond that of the parent.
    /// Any pages that are beyond the parent's range are initially all zeros.
    pub fn new_cow(parent: Vmo<R>, range: Range<usize>) -> Self {
        Self {
            flags: parent.flags(),
            parent,
            range,
            marker: PhantomData,
        }
    }

    /// Sets the VMO flags.
    ///
    /// Only the flags among `Self::CHILD_FLAGS_MASK` may be set through this
    /// method.
    ///
    /// To set `VmoFlags::RESIZABLE`, the child must be COW.
    ///
    /// The default value is `VmoFlags::empty()`.
    pub fn flags(mut self, flags: VmoFlags) -> Self {
        let inherited_flags = self.flags & Self::PARENT_FLAGS_MASK;
        self.flags = inherited_flags | (flags & Self::CHILD_FLAGS_MASK);
        self
    }
}

impl VmoChildOptions<Rights, VmoSliceChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights.
    pub fn alloc(self) -> Result<Vmo<Rights>> {
        let VmoChildOptions {
            parent,
            range,
            flags,
            ..
        } = self;
        let Vmo(parent_vmo_, parent_rights) = parent;
        let child_vmo_ = alloc_child_vmo_(parent_vmo_, range, flags, ChildType::Slice)?;
        Ok(Vmo(Arc::new(child_vmo_), parent_rights))
    }
}

impl VmoChildOptions<Rights, VmoCowChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights.
    pub fn alloc(self) -> Result<Vmo<Rights>> {
        let VmoChildOptions {
            parent,
            range,
            flags,
            ..
        } = self;
        let Vmo(parent_vmo_, parent_rights) = parent;
        let child_vmo_ = alloc_child_vmo_(parent_vmo_, range, flags, ChildType::Cow)?;
        Ok(Vmo(Arc::new(child_vmo_), parent_rights))
    }
}

impl<R: TRights> VmoChildOptions<TRightSet<R>, VmoSliceChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights.
    pub fn alloc(self) -> Result<Vmo<TRightSet<R>>> {
        let VmoChildOptions {
            parent,
            range,
            flags,
            ..
        } = self;
        let Vmo(parent_vmo_, parent_rights) = parent;
        let child_vmo_ = alloc_child_vmo_(parent_vmo_, range, flags, ChildType::Slice)?;
        Ok(Vmo(Arc::new(child_vmo_), parent_rights))
    }
}

impl<R: TRights> VmoChildOptions<TRightSet<R>, VmoCowChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights
    /// plus the Write right.
    pub fn alloc(self) -> Result<Vmo<TRightSet<SetExtendOp<R, Write>>>>
    where
        R: SetExtend<Write>,
        SetExtendOp<R, Write>: TRights,
    {
        let VmoChildOptions {
            parent,
            range,
            flags,
            ..
        } = self;
        let Vmo(parent_vmo_, _) = parent;
        let child_vmo_ = alloc_child_vmo_(parent_vmo_, range, flags, ChildType::Cow)?;
        let right = SetExtendOp::<R, Write>::new();
        Ok(Vmo(Arc::new(child_vmo_), TRightSet(right)))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ChildType {
    Cow,
    Slice,
}

fn alloc_child_vmo_(
    parent_vmo_: Arc<Vmo_>,
    range: Range<usize>,
    child_flags: VmoFlags,
    child_type: ChildType,
) -> Result<Vmo_> {
    let parent_page_idx_offset = range.start / PAGE_SIZE;
    let child_pages = parent_vmo_.clone_pages_for_child(child_type, child_flags, &range)?;
    let new_vmo = Vmo_ {
        pager: parent_vmo_.pager.clone(),
        flags: child_flags,
        pages: child_pages,
        page_idx_offset: parent_page_idx_offset + parent_vmo_.page_idx_offset(),
    };
    Ok(new_vmo)
}

/// A type to specify the "type" of a child, which is either a slice or a COW.
pub trait VmoChildType {}

/// A type to mark a child is slice.
#[derive(Copy, Clone, Debug)]
pub struct VmoSliceChild;
impl VmoChildType for VmoSliceChild {}

/// A type to mark a child is COW.
#[derive(Copy, Clone, Debug)]
pub struct VmoCowChild;
impl VmoChildType for VmoCowChild {}

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
    fn slice_child() {
        let parent = VmoOptions::<Full>::new(2 * PAGE_SIZE).alloc().unwrap();
        let parent_dup = parent.dup();
        let slice_child = VmoChildOptions::new_slice(parent_dup, 0..PAGE_SIZE)
            .alloc()
            .unwrap();
        // write parent, read child
        parent.write_val(1, &42u8).unwrap();
        assert_eq!(slice_child.read_val::<u8>(1).unwrap(), 42);
        // write child, read parent
        slice_child.write_val(99, &0x1234u32).unwrap();
        assert_eq!(parent.read_val::<u32>(99).unwrap(), 0x1234);
    }

    #[ktest]
    fn cow_child() {
        let parent = VmoOptions::<Full>::new(2 * PAGE_SIZE).alloc().unwrap();
        parent.write_val(1, &42u8).unwrap();
        parent.write_val(2, &16u8).unwrap();
        let parent_dup = parent.dup();
        let cow_child = VmoChildOptions::new_cow(parent_dup, 0..10 * PAGE_SIZE)
            .alloc()
            .unwrap();
        // Read child.
        assert_eq!(cow_child.read_val::<u8>(1).unwrap(), 42);
        assert_eq!(cow_child.read_val::<u8>(2).unwrap(), 16);
        // Write parent to trigger copy-on-write. read child and parent.
        parent.write_val(1, &64u8).unwrap();
        assert_eq!(parent.read_val::<u8>(1).unwrap(), 64);
        assert_eq!(cow_child.read_val::<u8>(1).unwrap(), 42);
        // Write child to trigger copy on write, read child and parent
        cow_child.write_val(2, &0x1234u32).unwrap();
        assert_eq!(cow_child.read_val::<u32>(2).unwrap(), 0x1234);
        assert_eq!(cow_child.read_val::<u8>(1).unwrap(), 42);
        assert_eq!(parent.read_val::<u8>(2).unwrap(), 16);
        assert_eq!(parent.read_val::<u8>(1).unwrap(), 64);
        // Write parent on already-copied page
        parent.write_val(1, &123u8).unwrap();
        assert_eq!(parent.read_val::<u8>(1).unwrap(), 123);
        assert_eq!(cow_child.read_val::<u8>(1).unwrap(), 42);
        // Write parent on not-copied page
        parent.write_val(2, &12345u32).unwrap();
        assert_eq!(parent.read_val::<u32>(2).unwrap(), 12345);
        assert_eq!(cow_child.read_val::<u32>(2).unwrap(), 0x1234);
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

    #[ktest]
    fn resize_cow() {
        let vmo = VmoOptions::<Full>::new(10 * PAGE_SIZE)
            .flags(VmoFlags::RESIZABLE)
            .alloc()
            .unwrap();

        let cow_child = VmoChildOptions::new_cow(vmo, 0..PAGE_SIZE).alloc().unwrap();

        cow_child.resize(2 * PAGE_SIZE).unwrap();
        assert_eq!(cow_child.size(), 2 * PAGE_SIZE);
    }
}
