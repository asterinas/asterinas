// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::prelude::*;

use aster_frame::vm::VmIo;

use aster_rights::{Rights, TRights};

use super::VmoRightsOp;
use super::{
    options::{VmoCowChild, VmoSliceChild},
    Vmo, VmoChildOptions,
};

impl Vmo<Rights> {
    /// Creates a new slice VMO through a set of VMO child options.
    ///
    /// # Example
    ///
    /// ```
    /// let parent = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    /// let child_size = parent.size();
    /// let child = parent.new_slice_child(0..child_size).alloc().unwrap();
    /// assert!(child.size() == child_size);
    /// ```
    ///
    /// For more details on the available options, see `VmoChildOptions`.
    ///
    /// # Access rights
    ///
    /// This method requires the Dup right.
    ///
    /// The new VMO child will be of the same capability flavor as the parent;
    /// so are the access rights.
    pub fn new_slice_child(
        &self,
        range: Range<usize>,
    ) -> Result<VmoChildOptions<Rights, VmoSliceChild>> {
        let dup_self = self.dup()?;
        Ok(VmoChildOptions::new_slice_rights(dup_self, range))
    }

    /// Creates a new COW VMO through a set of VMO child options.
    ///
    /// # Example
    ///
    /// ```
    /// let parent = VmoOptions::new(PAGE_SIZE).alloc().unwrap();
    /// let child_size = 2 * parent.size();
    /// let child = parent.new_cow_child(0..child_size).alloc().unwrap();
    /// assert!(child.size() == child_size);
    /// ```
    ///
    /// For more details on the available options, see `VmoChildOptions`.
    ///
    /// # Access rights
    ///
    /// This method requires the Dup right.
    ///
    /// The new VMO child will be of the same capability flavor as the parent.
    /// The child will be given the access rights of the parent
    /// plus the Write right.
    pub fn new_cow_child(
        &self,
        range: Range<usize>,
    ) -> Result<VmoChildOptions<Rights, VmoCowChild>> {
        let dup_self = self.dup()?;
        Ok(VmoChildOptions::new_cow(dup_self, range))
    }

    /// commit a page at specific offset
    pub fn commit_page(&self, offset: usize) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.commit_page(offset)
    }

    /// Commits the pages specified in the range (in bytes).
    ///
    /// The range must be within the size of the VMO.
    ///
    /// The start and end addresses will be rounded down and up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.commit(range)
    }

    /// Decommits the pages specified in the range (in bytes).
    ///
    /// The range must be within the size of the VMO.
    ///
    /// The start and end addresses will be rounded down and up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.decommit(range)
    }

    /// Resizes the VMO by giving a new size.
    ///
    /// The VMO must be resizable.
    ///
    /// The new size will be rounded up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    pub fn resize(&self, new_size: usize) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.resize(new_size)
    }

    /// Clears the specified range by writing zeros.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.clear(range)
    }

    /// Duplicates the capability.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    pub fn dup(&self) -> Result<Self> {
        self.check_rights(Rights::DUP)?;
        Ok(Self(self.0.clone(), self.1))
    }

    /// Restricts the access rights given the mask.
    pub fn restrict(mut self, mask: Rights) -> Self {
        self.1 |= mask;
        self
    }

    /// Converts to a static capability.
    pub fn to_static<R1: TRights>(self) -> Result<Vmo<R1>> {
        self.check_rights(Rights::from_bits(R1::BITS).ok_or(Error::new(Errno::EINVAL))?)?;
        Ok(Vmo(self.0, R1::new()))
    }
}

impl VmIo for Vmo<Rights> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> aster_frame::Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read_bytes(offset, buf)?;
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> aster_frame::Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write_bytes(offset, buf)?;
        Ok(())
    }
}

impl VmoRightsOp for Vmo<Rights> {
    fn rights(&self) -> Rights {
        self.1
    }

    /// Converts to a dynamic capability.
    fn to_dyn(self) -> Vmo<Rights> {
        let rights = self.rights();
        Vmo(self.0, rights)
    }
}
