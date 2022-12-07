use core::ops::Range;

use jinux_frame::prelude::Result;
use jinux_frame::vm::VmIo;
use jinux_rights_proc::require;

use crate::rights::*;

use super::VmoRightsOp;
use super::{
    options::{VmoCowChild, VmoSliceChild},
    Vmo, VmoChildOptions,
};

impl<R: TRights> Vmo<R> {
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
    #[require(R > Dup)]
    pub fn new_slice_child(
        &self,
        range: Range<usize>,
    ) -> Result<VmoChildOptions<R, VmoSliceChild>> {
        let dup_self = self.dup()?;
        Ok(VmoChildOptions::new_slice(dup_self, range))
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
    #[require(R > Dup)]
    pub fn new_cow_child(&self, range: Range<usize>) -> Result<VmoChildOptions<R, VmoCowChild>> {
        let dup_self = self.dup()?;
        Ok(VmoChildOptions::new_cow(dup_self, range))
    }

    /// Commit the pages specified in the range (in bytes).
    ///
    /// The range must be within the size of the VMO.
    ///
    /// The start and end addresses will be rounded down and up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    #[require(R > Write)]
    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        self.0.commit(range)
    }

    /// Decommit the pages specified in the range (in bytes).
    ///
    /// The range must be within the size of the VMO.
    ///
    /// The start and end addresses will be rounded down and up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    #[require(R > Write)]
    pub fn decommit(&self, range: Range<usize>) -> Result<()> {
        self.0.decommit(range)
    }

    /// Resize the VMO by giving a new size.
    ///
    /// The VMO must be resizable.
    ///
    /// The new size will be rounded up to page boundaries.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    #[require(R > Write)]
    pub fn resize(&self, new_size: usize) -> Result<()> {
        self.0.resize(new_size)
    }

    /// Clear the specified range by writing zeros.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    #[require(R > Write)]
    pub fn clear(&self, range: Range<usize>) -> Result<()> {
        self.0.clear(range)
    }

    /// Duplicate the capability.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    #[require(R > Dup)]
    pub fn dup(&self) -> Result<Self> {
        Ok(Vmo(self.0.clone(), self.1.clone()))
    }

    /// Strict the access rights.
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> Vmo<R1> {
        Vmo(self.0, R1::new())
    }
}

impl<R: TRights> VmIo for Vmo<R> {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read_bytes(offset, buf)
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write_bytes(offset, buf)
    }
}

impl<R: TRights> VmoRightsOp for Vmo<R> {
    fn rights(&self) -> Rights {
        Rights::from_bits(R::BITS).unwrap()
    }

    /// Converts to a dynamic capability.
    fn to_dyn(self) -> Vmo<Rights> {
        let rights = self.rights();
        Vmo(self.0, rights)
    }
}
