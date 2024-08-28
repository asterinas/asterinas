// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use aster_rights::{Rights, TRights};
use ostd::mm::{Frame, VmIo};

use super::{CommitFlags, Vmo, VmoRightsOp};
use crate::prelude::*;

impl Vmo<Rights> {
    /// Commits a page at specific offset
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
    pub fn commit(&self, range: Range<usize>) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
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
    pub(in crate::vm) fn operate_on_range<F>(&self, range: &Range<usize>, operate: F) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<Frame>) -> Result<()>,
    {
        self.check_rights(Rights::WRITE)?;
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

    /// Creates a new VMO that replicates the original capability, initially representing
    /// the same physical pages.
    /// Changes to the permissions and commits/replacements of internal pages in the original VMO
    /// and the new VMO will not affect each other.
    ///
    /// # Access rights
    ///
    /// The method requires the Dup right.
    pub fn dup_independent(&self) -> Result<Self> {
        self.check_rights(Rights::DUP | Rights::WRITE)?;
        Ok(Vmo(Arc::new(super::Vmo_::clone(&self.0)), self.1))
    }

    /// Replaces the page at the `page_idx` in the VMO with the input `page`.
    ///
    /// # Access rights
    ///
    /// The method requires the Write right.
    pub fn replace(&self, page: Frame, page_idx: usize) -> Result<()> {
        self.check_rights(Rights::WRITE)?;
        self.0.replace(page, page_idx)
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
    // TODO: Support efficient `write_vals()`
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
