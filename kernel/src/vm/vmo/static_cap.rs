// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use aster_rights::{Dup, Rights, TRightSet, TRights, Write};
use aster_rights_proc::require;
use ostd::mm::{Frame, VmIo};

use super::{CommitFlags, Vmo, VmoRightsOp};
use crate::prelude::*;

impl<R: TRights> Vmo<TRightSet<R>> {
    /// Commits a page at specific offset.
    pub fn commit_page(&self, offset: usize) -> Result<Frame> {
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
    #[require(R > Write)]
    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        self.0.operate_on_range(
            &range,
            |commit_fn| commit_fn().map(|_| ()),
            CommitFlags::empty(),
        )?;
        Ok(())
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    #[require(R > Write)]
    pub(in crate::vm) fn operate_on_range<F>(&self, range: &Range<usize>, operate: F) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<Frame>) -> Result<()>,
    {
        self.0
            .operate_on_range(range, operate, CommitFlags::empty())
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
    pub fn dup(&self) -> Self {
        Vmo(self.0.clone(), self.1)
    }

    /// Creates a new VMO that replicates the original capability, initially representing
    /// the same physical pages.
    /// Changes to the permissions and commits/replacements of internal pages in the original VMO
    /// and the new VMO will not affect each other.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    #[require(R > Dup | Write)]
    pub fn dup_independent(&self) -> Self {
        Vmo(Arc::new(super::Vmo_::clone(&self.0)), self.1)
    }

    /// Replaces the page at the `page_idx` in the VMO with the input `page`.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    #[require(R > Write)]
    pub fn replace(&self, page: Frame, page_idx: usize) -> Result<()> {
        self.0.replace(page, page_idx)
    }

    /// Strict the access rights.
    #[require(R > R1)]
    pub fn restrict<R1: TRights>(self) -> Vmo<TRightSet<R1>> {
        Vmo(self.0, TRightSet(R1::new()))
    }
}

impl<R: TRights> VmIo for Vmo<TRightSet<R>> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        self.check_rights(Rights::READ)?;
        self.0.read(offset, writer)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.write(offset, reader)?;
        Ok(())
    }
}

impl<R: TRights> VmoRightsOp for Vmo<TRightSet<R>> {
    fn rights(&self) -> Rights {
        Rights::from_bits(R::BITS).unwrap()
    }

    /// Converts to a dynamic capability.
    fn to_dyn(self) -> Vmo<Rights> {
        let rights = self.rights();
        Vmo(self.0, rights)
    }
}
