// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::prelude::*;
use aster_frame::vm::VmIo;
use aster_rights::{Dup, Rights, TRightSet, TRights};
use aster_rights_proc::require;

use crate::vm::{page_fault_handler::PageFaultHandler, vmo::Vmo};

use super::{
    options::VmarChildOptions, vm_mapping::VmarMapOptions, VmPerms, Vmar, VmarRightsOp, Vmar_,
};

impl<R: TRights> Vmar<TRightSet<R>> {
    /// Creates a root VMAR.
    ///
    /// # Access rights
    ///
    /// A root VMAR is initially given full access rights.
    pub fn new_root() -> Self {
        let inner = Vmar_::new_root();
        let rights = R::new();
        Self(inner, TRightSet(rights))
    }

    /// Maps the given VMO into the VMAR through a set of VMAR mapping options.
    ///
    /// # Example
    ///
    /// ```
    /// use aster_std::prelude::*;
    /// use aster_std::vm::{PAGE_SIZE, Vmar, VmoOptions};
    ///
    /// let vmar = Vmar::<RightsWrapper<Full>>::new().unwrap();
    /// let vmo = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    /// let target_vaddr = 0x1234000;
    /// let real_vaddr = vmar
    ///     // Map the VMO to create a read-only mapping
    ///     .new_map(vmo, VmPerms::READ)
    ///     // Provide an optional offset for the mapping inside the VMAR
    ///     .offset(target_vaddr)
    ///     .build()
    ///     .unwrap();
    /// assert!(real_vaddr == target_vaddr);
    /// ```
    ///
    /// For more details on the available options, see `VmarMapOptions`.
    ///
    /// # Access rights
    ///
    /// This method requires the following access rights:
    /// 1. The VMAR contains the rights corresponding to the memory permissions of
    /// the mapping. For example, if `perms` contains `VmPerm::WRITE`,
    /// then the VMAR must have the Write right.
    /// 2. Similarly, the VMO contains the rights corresponding to the memory
    /// permissions of the mapping.  
    ///
    /// Memory permissions may be changed through the `protect` method,
    /// which ensures that any updated memory permissions do not go beyond
    /// the access rights of the underlying VMOs.
    #[require(R > Dup)]
    pub fn new_map(
        &self,
        vmo: Vmo<Rights>,
        perms: VmPerms,
    ) -> Result<VmarMapOptions<TRightSet<R>, Rights>> {
        let dup_self = self.dup()?;
        Ok(VmarMapOptions::new(dup_self, vmo, perms))
    }

    /// Creates a new child VMAR through a set of VMAR child options.
    ///
    /// # Example
    ///
    /// ```
    /// let parent = Vmar::new().unwrap();
    /// let child_size = 10 * PAGE_SIZE;
    /// let child = parent.new_child(child_size).alloc().unwrap();
    /// assert!(child.size() == child_size);
    /// ```
    ///
    /// For more details on the available options, see `VmarChildOptions`.
    ///
    /// # Access rights
    ///
    /// This method requires the Dup right.
    ///
    /// The new VMAR child will be of the same capability class and
    /// access rights as the parent.
    #[require(R > Dup)]
    pub fn new_child(&self, size: usize) -> Result<VmarChildOptions<TRightSet<R>>> {
        let dup_self = self.dup()?;
        Ok(VmarChildOptions::new(dup_self, size))
    }

    /// Change the permissions of the memory mappings in the specified range.
    ///
    /// The range's start and end addresses must be page-aligned.
    /// Also, the range must be completely mapped.
    ///
    /// # Access rights
    ///
    /// The VMAR must have the rights corresponding to the specified memory
    /// permissions.
    ///
    /// The mappings overlapped with the specified range must be backed by
    /// VMOs whose rights contain the rights corresponding to the specified
    /// memory permissions.
    pub fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        self.check_rights(perms.into())?;
        self.0.protect(perms, range)
    }

    /// clear all mappings and children vmars.
    /// After being cleared, this vmar will become an empty vmar
    pub fn clear(&self) -> Result<()> {
        self.0.clear_root_vmar()
    }

    /// Destroy a VMAR, including all its mappings and children VMARs.
    ///
    /// After being destroyed, the VMAR becomes useless and returns errors
    /// for most of its methods.
    pub fn destroy_all(&self) -> Result<()> {
        self.0.destroy_all()
    }

    /// Destroy all mappings and children VMARs that fall within the specified
    /// range in bytes.
    ///
    /// The range's start and end addresses must be page-aligned.
    ///
    /// Mappings may fall partially within the range; only the overlapped
    /// portions of the mappings are unmapped.
    /// As for children VMARs, they must be fully within the range.
    /// All children VMARs that fall within the range get their `destroy` methods
    /// called.
    pub fn destroy(&self, range: Range<usize>) -> Result<()> {
        self.0.destroy(range)
    }

    /// Duplicate the capability.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    #[require(R > Dup)]
    pub fn dup(&self) -> Result<Self> {
        Ok(Vmar(self.0.clone(), self.1))
    }

    /// Creates a new root VMAR whose content is inherited from another
    /// using copy-on-write (COW) technique.
    ///
    /// # Access rights
    ///
    /// The method requires the Read right.
    pub fn fork_from<R1>(vmar: &Vmar<R1>) -> Result<Self> {
        vmar.check_rights(Rights::READ)?;
        let vmar_ = vmar.0.new_cow_root()?;
        Ok(Vmar(vmar_, TRightSet(R::new())))
    }

    /// Strict the access rights.
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> Vmar<R1> {
        Vmar(self.0, R1::new())
    }

    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights().contains(rights) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EACCES, "check rights failed");
        }
    }
}

impl<R: TRights> VmIo for Vmar<TRightSet<R>> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> aster_frame::Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read(offset, buf)?;
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> aster_frame::Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write(offset, buf)?;
        Ok(())
    }
}

impl<R: TRights> PageFaultHandler for Vmar<TRightSet<R>> {
    fn handle_page_fault(
        &self,
        page_fault_addr: Vaddr,
        not_present: bool,
        write: bool,
    ) -> Result<()> {
        if write {
            self.check_rights(Rights::WRITE)?;
        } else {
            self.check_rights(Rights::READ)?;
        }
        self.0
            .handle_page_fault(page_fault_addr, not_present, write)
    }
}

impl<R: TRights> VmarRightsOp for Vmar<TRightSet<R>> {
    fn rights(&self) -> Rights {
        Rights::from_bits(R::BITS).unwrap()
    }
}
