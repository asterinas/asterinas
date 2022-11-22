//! Options for allocating root and child VMOs.

use core::marker::PhantomData;
use core::ops::Range;

use alloc::sync::Arc;
use jinux_frame::prelude::Result;
use jinux_frame::vm::Paddr;
use jinux_rights_proc::require;

use crate::rights::{Dup, Rights, TRights};

use super::{Pager, Vmo, VmoFlags};

/// Options for allocating a root VMO.
///
/// # Examples
///
/// Creating a VMO as a _dynamic_ capability with full access rights:
/// ```
/// use kxo_std::vm::{PAGE_SIZE, VmoOptions};
///
/// let vmo = VmoOptions::new(PAGE_SIZE)
///     .alloc()
///     .unwrap();
/// ```
///
/// Creating a VMO as a _static_ capability with all access rights:
/// ```
/// use jinux_std::prelude::*;
/// use kxo_std::vm::{PAGE_SIZE, VmoOptions};
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
/// use jinux_std::vm::{PAGE_SIZE, VmoOptions, VmoFlags};
///
/// let vmo = VmoOptions::new(10 * PAGE_SIZE)
///     .flags(VmoFlags::RESIZABLE)
///     .alloc()
///     .unwrap();
/// ```
pub struct VmoOptions<R = Rights> {
    size: usize,
    paddr: Option<Paddr>,
    flags: VmoFlags,
    rights: R,
    // supplier: Option<Arc<dyn FrameSupplier>>,
}

impl<R> VmoOptions<R> {
    /// Creates a default set of options with the specified size of the VMO
    /// (in bytes).
    ///
    /// The size of the VMO will be rounded up to align with the page size.
    pub fn new(size: usize) -> Self {
        todo!()
    }

    /// Sets the starting physical address of the VMO.
    ///
    /// By default, this option is not set.
    ///
    /// If this option is set, then the underlying pages of VMO must be contiguous.
    /// So `VmoFlags::IS_CONTIGUOUS` will be set automatically.
    pub fn paddr(mut self, paddr: Paddr) -> Self {
        todo!()
    }

    /// Sets the VMO flags.
    ///
    /// The default value is `VmoFlags::empty()`.
    ///
    /// For more information about the flags, see `VmoFlags`.
    pub fn flags(mut self, flags: VmoFlags) -> Self {
        todo!()
    }

    /// Sets the pager of the VMO.
    pub fn pager(mut self, pager: Arc<dyn Pager>) -> Self {
        todo!()
    }
}

impl VmoOptions<Rights> {
    /// Allocates the VMO according to the specified options.
    ///
    /// # Access rights
    ///
    /// The VMO is initially assigned full access rights.
    pub fn alloc(mut self) -> Result<Vmo<Rights>> {
        todo!()
    }
}

impl<R: TRights> VmoOptions<R> {
    /// Allocates the VMO according to the specified options.
    ///
    /// # Access rights
    ///
    /// The VMO is initially assigned the access rights represented
    /// by `R: TRights`.
    pub fn alloc(mut self) -> Result<Vmo<R>> {
        todo!()
    }
}

/// Options for allocating a child VMO out of a parent VMO.
///
/// # Examples
///
/// A child VMO created from a parent VMO of _dynamic_ capability is also a
/// _dynamic_ capability.
/// ```
/// use kxo_std::vm::{PAGE_SIZE, VmoOptions};
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
/// use jinux_std::prelude::*;
/// use jinux_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use kxo_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use jinux_std::vm::{PAGE_SIZE, VmoOptions, VmoChildOptions};
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
/// use kxo_std::vm::{PAGE_SIZE, VmoOptions};
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

impl<R: TRights> VmoChildOptions<R, VmoSliceChild> {
    /// Creates a default set of options for creating a slice VMO child.
    ///
    /// A slice child of a VMO, which has direct access to a range of memory
    /// pages in the parent VMO. In other words, any updates of the parent will
    /// reflect on the child, and vice versa.
    ///
    /// The range of a child must be within that of the parent.
    #[require(R > Dup)]
    pub fn new_slice(parent: Vmo<R>, range: Range<usize>) -> Self {
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

impl<R> VmoChildOptions<R, VmoCowChild> {
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
}

impl<R, C> VmoChildOptions<R, C> {
    /// Flags that a VMO child inherits from its parent.
    pub const PARENT_FLAGS_MASK: VmoFlags =
        VmoFlags::from_bits(VmoFlags::CONTIGUOUS.bits | VmoFlags::DMA.bits).unwrap();
    /// Flags that a VMO child may differ from its parent.
    pub const CHILD_FLAGS_MASK: VmoFlags = VmoFlags::RESIZABLE;

    /// Sets the VMO flags.
    ///
    /// Only the flags among `Self::CHILD_FLAGS_MASK` may be set through this
    /// method.
    ///
    /// To set `VmoFlags::RESIZABLE`, the child must be COW.
    ///
    /// The default value is `VmoFlags::empty()`.
    pub fn flags(mut self, flags: VmoFlags) -> Self {
        self.flags = flags & Self::CHILD_FLAGS_MASK;
        self
    }
}

impl<C> VmoChildOptions<Rights, C> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights.
    pub fn alloc(mut self) -> Result<Vmo<Rights>> {
        todo!()
    }
}

impl<R: TRights> VmoChildOptions<R, VmoSliceChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights.
    pub fn alloc(mut self) -> Result<Vmo<R>> {
        todo!()
    }
}

impl<R: TRights> VmoChildOptions<R, VmoCowChild> {
    /// Allocates the child VMO.
    ///
    /// # Access rights
    ///
    /// The child VMO is initially assigned all the parent's access rights
    /// plus the Write right.
    pub fn alloc<R1>(mut self) -> Result<Vmo<R1>>
    where
        R1: TRights, // TODO: R1 must contain the Write right. To do so at the type level,
                     // we need to implement a type-level operator
                     // (say, `TRightsExtend(L, F)`)
                     // that may extend a list (`L`) of type-level flags with an extra flag `F`.
                     // TRightsExtend<R, Write>
    {
        todo!()
    }

    // original:
    // pub fn alloc<R1>(mut self) -> Result<Vmo<R1>>
    // where
    //     // TODO: R1 must contain the Write right. To do so at the type level,
    //     // we need to implement a type-level operator
    //     // (say, `TRightsExtend(L, F)`)
    //     // that may extend a list (`L`) of type-level flags with an extra flag `F`.
    //     R1: R // TRightsExtend<R, Write>
    // {
    //     todo!()
    // }
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
