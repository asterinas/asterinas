//! Options for allocating child VMARs.

use jinux_frame::config::PAGE_SIZE;
use jinux_frame::{Error, Result};

use super::Vmar;

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
            debug_assert!(align % PAGE_SIZE == 0);
            debug_assert!(align.is_power_of_two());
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
