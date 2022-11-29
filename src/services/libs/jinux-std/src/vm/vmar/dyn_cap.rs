use core::ops::Range;

use alloc::sync::Arc;
use jinux_frame::prelude::Result;
use jinux_frame::{vm::VmIo, Error};

use crate::{rights::Rights, vm::vmo::Vmo};

use super::{
    options::{VmarChildOptions, VmarMapOptions},
    VmPerms, Vmar, Vmar_,
};

impl Vmar<Rights> {
    /// Creates a root VMAR.
    pub fn new() -> Result<Self> {
        let inner = Arc::new(Vmar_::new()?);
        let rights = Rights::all();
        let new_self = Self(inner, rights);
        Ok(new_self)
    }

    /// Maps the given VMO into the VMAR through a set of VMAR mapping options.
    ///
    /// # Example
    ///
    /// ```
    /// use jinux_std::prelude::*;
    /// use jinux_std::vm::{PAGE_SIZE, Vmar, VmoOptions};
    ///
    /// let vmar = Vmar::new().unwrap();
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
    pub fn new_map(
        &self,
        vmo: Vmo<Rights>,
        perms: VmPerms,
    ) -> Result<VmarMapOptions<Rights, Rights>> {
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
    pub fn new_child(&self, size: usize) -> Result<VmarChildOptions<Rights>> {
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

    /// Duplicates the capability.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    pub fn dup(&self) -> Result<Self> {
        self.check_rights(Rights::DUP)?;
        todo!()
    }

    /// Returns the access rights.
    pub fn rights(&self) -> Rights {
        self.1
    }

    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.1.contains(rights) {
            Ok(())
        } else {
            Err(Error::AccessDenied)
        }
    }
}

impl VmIo for Vmar<Rights> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read(offset, buf)
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write(offset, buf)
    }
}
