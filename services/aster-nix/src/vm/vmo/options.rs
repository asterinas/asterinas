// SPDX-License-Identifier: MPL-2.0

//! Options for allocating root and child VMOs.

use core::marker::PhantomData;
use core::ops::Range;

use align_ext::AlignExt;
use aster_frame::vm::{VmAllocOptions, VmFrame};
use aster_rights_proc::require;
use typeflags_util::{SetExtend, SetExtendOp};

use crate::prelude::*;

use crate::vm::vmo::get_inherited_frames_from_parent;
use crate::vm::vmo::{VmoInner, Vmo_};
use aster_rights::{Dup, Rights, TRightSet, TRights, Write};

use super::VmoRightsOp;
use super::{Pager, Vmo, VmoFlags};

/// Options for allocating a root VMO.
///
/// # Examples
///
/// Creating a VMO as a _dynamic_ capability with full access rights:
/// ```
/// use aster_std::vm::{PAGE_SIZE, VmoOptions};
///
/// let vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// ```
///
/// Creating a VMO as a _static_ capability with all access rights:
/// ```
/// use aster_std::prelude::*;
/// use aster_std::vm::{PAGE_SIZE, VmoOptions};
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
/// use aster_std::vm::{PAGE_SIZE, VmoOptions, VmoFlags};
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
    let committed_pages = committed_pages_if_continuous(flags, size)?;
    let vmo_inner = VmoInner {
        pager,
        size,
        committed_pages,
        inherited_pages: None,
        is_cow: false,
    };
    Ok(Vmo_ {
        flags,
        inner: Mutex::new(vmo_inner),
    })
}

fn committed_pages_if_continuous(flags: VmoFlags, size: usize) -> Result<BTreeMap<usize, VmFrame>> {
    if flags.contains(VmoFlags::CONTIGUOUS) {
        // if the vmo is continuous, we need to allocate frames for the vmo
        let frames_num = size / PAGE_SIZE;
        let frames = VmAllocOptions::new(frames_num)
            .is_contiguous(true)
            .alloc()?;
        let mut committed_pages = BTreeMap::new();
        for (idx, frame) in frames.into_iter().enumerate() {
            committed_pages.insert(idx * PAGE_SIZE, frame);
        }
        Ok(committed_pages)
    } else {
        // otherwise, we wait for the page is read or write
        Ok(BTreeMap::new())
    }
}

/// Options for allocating a child VMO out of a parent VMO.
///
/// # Examples
///
/// A child VMO created from a parent VMO of _dynamic_ capability is also a
/// _dynamic_ capability.
/// ```
/// use aster_std::vm::{PAGE_SIZE, VmoOptions};
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
/// use aster_std::prelude::*;
/// use aster_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use aster_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use aster_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use aster_std::vm::{PAGE_SIZE, VmoOptions};
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
            flags: parent.flags() & Self::PARENT_FLAGS_MASK,
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
            flags: parent.flags() & Self::PARENT_FLAGS_MASK,
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
enum ChildType {
    Cow,
    Slice,
}

fn alloc_child_vmo_(
    parent_vmo_: Arc<Vmo_>,
    range: Range<usize>,
    child_flags: VmoFlags,
    child_type: ChildType,
) -> Result<Vmo_> {
    let child_vmo_start = range.start;
    let child_vmo_end = range.end;
    debug_assert!(child_vmo_start % PAGE_SIZE == 0);
    debug_assert!(child_vmo_end % PAGE_SIZE == 0);
    if child_vmo_start % PAGE_SIZE != 0 || child_vmo_end % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "vmo range does not aligned with PAGE_SIZE");
    }
    let parent_vmo_size = parent_vmo_.size();

    let is_cow = {
        let parent_vmo_inner = parent_vmo_.inner.lock();
        match child_type {
            ChildType::Slice => {
                // A slice child should be inside parent vmo's range
                debug_assert!(child_vmo_end <= parent_vmo_inner.size);
                if child_vmo_end > parent_vmo_inner.size {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "slice child vmo cannot exceed parent vmo's size"
                    );
                }
                false
            }
            ChildType::Cow => {
                // A copy on Write child should intersect with parent vmo
                debug_assert!(range.start <= parent_vmo_inner.size);
                if range.start > parent_vmo_inner.size {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "COW vmo should overlap with its parent"
                    );
                }
                true
            }
        }
    };
    let parent_page_idx_offset = range.start / PAGE_SIZE;
    let inherited_end = range.end.min(parent_vmo_size);
    let cow_size = if inherited_end >= range.start {
        inherited_end - range.start
    } else {
        0
    };
    let num_pages = cow_size / PAGE_SIZE;
    let inherited_pages =
        get_inherited_frames_from_parent(parent_vmo_, num_pages, parent_page_idx_offset, is_cow);
    let vmo_inner = VmoInner {
        pager: None,
        size: child_vmo_end - child_vmo_start,
        committed_pages: BTreeMap::new(),
        inherited_pages: Some(inherited_pages),
        is_cow,
    };
    Ok(Vmo_ {
        flags: child_flags,
        inner: Mutex::new(vmo_inner),
    })
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

#[if_cfg_ktest]
mod test {
    use super::*;
    use aster_frame::vm::VmIo;
    use aster_rights::Full;

    #[ktest]
    fn alloc_vmo() {
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap();
        assert!(vmo.size() == PAGE_SIZE);
        // the vmo is zeroed once allocated
        assert!(vmo.read_val::<usize>(0).unwrap() == 0);
    }

    #[ktest]
    fn alloc_continuous_vmo() {
        let vmo = VmoOptions::<Full>::new(10 * PAGE_SIZE)
            .flags(VmoFlags::CONTIGUOUS)
            .alloc()
            .unwrap();
        assert!(vmo.size() == 10 * PAGE_SIZE);
    }

    #[ktest]
    fn write_and_read() {
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap();
        let val = 42u8;
        // write val
        vmo.write_val(111, &val).unwrap();
        let read_val: u8 = vmo.read_val(111).unwrap();
        assert!(val == read_val);
        // bit endian
        vmo.write_bytes(222, &[0x12, 0x34, 0x56, 0x78]).unwrap();
        let read_val: u32 = vmo.read_val(222).unwrap();
        assert!(read_val == 0x78563412)
    }

    #[ktest]
    fn slice_child() {
        let parent = VmoOptions::<Full>::new(2 * PAGE_SIZE).alloc().unwrap();
        let parent_dup = parent.dup().unwrap();
        let slice_child = VmoChildOptions::new_slice(parent_dup, 0..PAGE_SIZE)
            .alloc()
            .unwrap();
        // write parent, read child
        parent.write_val(1, &42u8).unwrap();
        assert!(slice_child.read_val::<u8>(1).unwrap() == 42);
        // write child, read parent
        slice_child.write_val(99, &0x1234u32).unwrap();
        assert!(parent.read_val::<u32>(99).unwrap() == 0x1234);
    }

    #[ktest]
    fn cow_child() {
        let parent = VmoOptions::<Full>::new(2 * PAGE_SIZE).alloc().unwrap();
        let parent_dup = parent.dup().unwrap();
        let cow_child = VmoChildOptions::new_cow(parent_dup, 0..10 * PAGE_SIZE)
            .alloc()
            .unwrap();
        // write parent, read child
        parent.write_val(1, &42u8).unwrap();
        assert!(cow_child.read_val::<u8>(1).unwrap() == 42);
        // write child to trigger copy on write, read child and parent
        cow_child.write_val(99, &0x1234u32).unwrap();
        assert!(cow_child.read_val::<u32>(99).unwrap() == 0x1234);
        assert!(cow_child.read_val::<u32>(1).unwrap() == 42);
        assert!(parent.read_val::<u32>(99).unwrap() == 0);
        assert!(parent.read_val::<u32>(1).unwrap() == 42);
        // write parent on already-copied page
        parent.write_val(10, &123u8).unwrap();
        assert!(parent.read_val::<u32>(10).unwrap() == 123);
        assert!(cow_child.read_val::<u32>(10).unwrap() == 0);
        // write parent on not-copied page
        parent.write_val(PAGE_SIZE + 10, &12345u32).unwrap();
        assert!(parent.read_val::<u32>(PAGE_SIZE + 10).unwrap() == 12345);
        assert!(cow_child.read_val::<u32>(PAGE_SIZE + 10).unwrap() == 12345);
    }

    #[ktest]
    fn resize() {
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE)
            .flags(VmoFlags::RESIZABLE)
            .alloc()
            .unwrap();
        vmo.write_val(10, &42u8).unwrap();
        vmo.resize(2 * PAGE_SIZE).unwrap();
        assert!(vmo.size() == 2 * PAGE_SIZE);
        assert!(vmo.read_val::<u8>(10).unwrap() == 42);
        vmo.write_val(PAGE_SIZE + 20, &123u8).unwrap();
        vmo.resize(PAGE_SIZE).unwrap();
        assert!(vmo.read_val::<u8>(10).unwrap() == 42);
    }
}
