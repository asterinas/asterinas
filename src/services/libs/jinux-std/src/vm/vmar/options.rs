//! Options for allocating child VMARs and creating mappings.

use jinux_frame::{config::PAGE_SIZE, vm::Vaddr};
use jinux_frame::{Error, Result};

use crate::vm::vmo::Vmo;

use super::{VmPerms, Vmar};

/// Options for allocating a child VMAR, which must not overlap with any
/// existing mappings or child VMARs.
///
/// # Examples
///
/// A child VMAR created from a parent VMAR of _dynamic_ capability is also a
/// _dynamic_ capability.
/// ```
/// use jinux_std::vm::{PAGE_SIZE, Vmar};
///
/// let parent_vmar = Vmar::new();
/// let child_size = 10 * PAGE_SIZE;
/// let child_vmar = parent_vmar
///     .new_child(child_size)
///     .alloc()
///     .unwrap();
/// assert!(child_vmar.rights() == parent_vmo.rights());
/// assert!(child_vmar.size() == child_size);
/// ```
///
/// A child VMAR created from a parent VMAR of _static_ capability is also a
/// _static_ capability.
/// ```
/// use jinux_std::prelude::*;
/// use jinux_std::vm::{PAGE_SIZE, Vmar};
///
/// let parent_vmar: Vmar<Full> = Vmar::new();
/// let child_size = 10 * PAGE_SIZE;
/// let child_vmar = parent_vmar
///     .new_child(child_size)
///     .alloc()
///     .unwrap();
/// assert!(child_vmar.rights() == parent_vmo.rights());
/// assert!(child_vmar.size() == child_size);
/// ```
pub struct VmarChildOptions<R> {
    parent: Vmar<R>,
    size: usize,
    offset: Option<usize>,
    align: Option<usize>,
}

impl<R> VmarChildOptions<R> {
    /// Creates a default set of options with the specified size of the VMAR
    /// (in bytes).
    ///
    /// The size of the VMAR will be rounded up to align with the page size.
    pub fn new(parent: Vmar<R>, size: usize) -> Self {
        Self {
            parent,
            size,
            offset: None,
            align: None,
        }
    }

    /// Set the alignment of the child VMAR.
    ///
    /// By default, the alignment is the page size.
    ///
    /// The alignment must be a power of two and a multiple of the page size.
    pub fn align(mut self, align: usize) -> Self {
        self.align = Some(align);
        self
    }

    /// Sets the offset of the child VMAR.
    ///
    /// If not set, the system will choose an offset automatically.
    ///
    /// The offset must satisfy the alignment requirement.
    /// Also, the child VMAR's range `[offset, offset + size)` must be within
    /// the VMAR.
    ///
    /// If not specified,
    ///
    /// The offset must be page-aligned.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Allocates the child VMAR according to the specified options.
    ///
    /// The new child VMAR
    ///
    /// # Access rights
    ///
    /// The child VMAR is initially assigned all the parent's access rights.
    pub fn alloc(self) -> Result<Vmar<R>> {
        // check align
        let align = if let Some(align) = self.align {
            if align % PAGE_SIZE != 0 || !align.is_power_of_two() {
                return Err(Error::InvalidArgs);
            }
            align
        } else {
            PAGE_SIZE
        };
        // check size
        if self.size % align != 0 {
            return Err(Error::InvalidArgs);
        }
        // check offset
        let root_vmar_offset = if let Some(offset) = self.offset {
            if offset % PAGE_SIZE != 0 {
                return Err(Error::InvalidArgs);
            }
            let root_vmar_offset = offset + self.parent.base();
            if root_vmar_offset % align != 0 {
                return Err(Error::InvalidArgs);
            }
            Some(root_vmar_offset)
        } else {
            None
        };
        let child_vmar_ = self
            .parent
            .0
            .alloc_child_vmar(root_vmar_offset, self.size, align)?;
        let child_vmar = Vmar(child_vmar_, self.parent.1);
        Ok(child_vmar)
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<R1, R2> {
    parent: Vmar<R1>,
    vmo: Vmo<R2>,
    perms: VmPerms,
    vmo_offset: usize,
    size: usize,
    offset: Option<usize>,
    align: usize,
    can_overwrite: bool,
}

impl<R1, R2> VmarMapOptions<R1, R2> {
    /// Creates a default set of options with the VMO and the memory access
    /// permissions.
    ///
    /// The VMO must have access rights that correspond to the memory
    /// access permissions. For example, if `perms` contains `VmPerm::Write`,
    /// then `vmo.rights()` should contain `Rights::WRITE`.
    pub fn new(parent: Vmar<R1>, vmo: Vmo<R2>, perms: VmPerms) -> Self {
        let size = vmo.size();
        Self {
            parent,
            vmo,
            perms,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            can_overwrite: false,
        }
    }

    /// Sets the offset of the first memory page in the VMO that is to be
    /// mapped into the VMAR.
    ///
    /// The offset must be page-aligned and within the VMO.
    ///
    /// The default value is zero.
    pub fn vmo_offset(mut self, offset: usize) -> Self {
        self.vmo_offset = offset;
        self
    }

    /// Sets the size of the mapping.
    ///
    /// The size of a mapping may not be equal to that of the VMO.
    /// For example, it is ok to create a mapping whose size is larger than
    /// that of the VMO, although one cannot read from or write to the
    /// part of the mapping that is not backed by the VMO.
    /// So you may wonder: what is the point of supporting such _oversized_
    /// mappings?  The reason is two-fold.
    /// 1. VMOs are resizable. So even if a mapping is backed by a VMO whose
    /// size is equal to that of the mapping initially, we cannot prevent
    /// the VMO from shrinking.
    /// 2. Mappings are not allowed to overlap by default. As a result,
    /// oversized mappings can serve as a placeholder to prevent future
    /// mappings from occupying some particular address ranges accidentally.
    ///
    /// The default value is the size of the VMO.
    pub fn size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Sets the mapping's alignment.
    ///
    /// The default value is the page size.
    ///
    /// The provided alignment must be a power of two and a multiple of the
    /// page size.
    pub fn align(mut self, align: usize) -> Self {
        self.align = align;
        self
    }

    /// Sets the mapping's offset inside the VMAR.
    ///
    /// The offset must satisfy the alignment requirement.
    /// Also, the mapping's range `[offset, offset + size)` must be within
    /// the VMAR.
    ///
    /// If not set, the system will choose an offset automatically.
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Sets whether the mapping can overwrite existing mappings.
    ///
    /// The default value is false.
    ///
    /// If this option is set to true, then the `offset` option must be
    /// set.
    pub fn can_overwrite(mut self, can_overwrite: bool) -> Self {
        self.can_overwrite = can_overwrite;
        self
    }

    /// Creates the mapping.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub fn build(mut self) -> Result<Vaddr> {
        todo!()
    }
}
