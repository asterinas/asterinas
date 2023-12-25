//! Options for allocating child VMARs.

use aster_frame::config::PAGE_SIZE;
use aster_frame::{Error, Result};

use super::Vmar;

/// Options for allocating a child VMAR, which must not overlap with any
/// existing mappings or child VMARs.
///
/// # Examples
///
/// A child VMAR created from a parent VMAR of _dynamic_ capability is also a
/// _dynamic_ capability.
/// ```
/// use aster_std::vm::{PAGE_SIZE, Vmar};
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
/// use aster_std::prelude::*;
/// use aster_std::vm::{PAGE_SIZE, Vmar};
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

#[if_cfg_ktest]
mod test {
    use super::*;
    use crate::vm::page_fault_handler::PageFaultHandler;
    use crate::vm::perms::VmPerms;
    use crate::vm::vmo::VmoRightsOp;
    use crate::vm::{vmar::ROOT_VMAR_HIGHEST_ADDR, vmo::VmoOptions};
    use aster_frame::vm::VmIo;
    use aster_rights::Full;

    #[ktest]
    fn root_vmar() {
        let vmar = Vmar::<Full>::new_root();
        assert!(vmar.size() == ROOT_VMAR_HIGHEST_ADDR);
    }

    #[ktest]
    fn child_vmar() {
        let root_vmar = Vmar::<Full>::new_root();
        let root_vmar_dup = root_vmar.dup().unwrap();
        let child_vmar = VmarChildOptions::new(root_vmar_dup, 10 * PAGE_SIZE)
            .alloc()
            .unwrap();
        assert!(child_vmar.size() == 10 * PAGE_SIZE);
        let root_vmar_dup = root_vmar.dup().unwrap();
        let second_child = VmarChildOptions::new(root_vmar_dup, 9 * PAGE_SIZE)
            .alloc()
            .unwrap();
        let root_vmar_dup = root_vmar.dup().unwrap();
        assert!(VmarChildOptions::new(root_vmar_dup, 9 * PAGE_SIZE)
            .offset(11 * PAGE_SIZE)
            .alloc()
            .is_err());
    }

    #[ktest]
    fn map_vmo() {
        let root_vmar = Vmar::<Full>::new_root();
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap().to_dyn();
        let perms = VmPerms::READ | VmPerms::WRITE;
        let map_offset = 0x1000_0000;
        let vmo_dup = vmo.dup().unwrap();
        root_vmar
            .new_map(vmo_dup, perms)
            .unwrap()
            .offset(map_offset)
            .build()
            .unwrap();
        root_vmar.write_val(map_offset, &100u8).unwrap();
        assert!(root_vmar.read_val::<u8>(map_offset).unwrap() == 100);
        let another_map_offset = 0x1100_0000;
        let vmo_dup = vmo.dup().unwrap();
        root_vmar
            .new_map(vmo_dup, perms)
            .unwrap()
            .offset(another_map_offset)
            .build()
            .unwrap();
        assert!(root_vmar.read_val::<u8>(another_map_offset).unwrap() == 100);
    }

    #[ktest]
    fn handle_page_fault() {
        const OFFSET: usize = 0x1000_0000;
        let root_vmar = Vmar::<Full>::new_root();
        // the page is not mapped by a vmo
        assert!(root_vmar.handle_page_fault(OFFSET, true, true).is_err());
        // the page is mapped READ
        let vmo = VmoOptions::<Full>::new(PAGE_SIZE).alloc().unwrap().to_dyn();
        let perms = VmPerms::READ;
        let vmo_dup = vmo.dup().unwrap();
        root_vmar
            .new_map(vmo_dup, perms)
            .unwrap()
            .offset(OFFSET)
            .build()
            .unwrap();
        root_vmar.handle_page_fault(OFFSET, true, false).unwrap();
    }
}
