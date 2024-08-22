// SPDX-License-Identifier: MPL-2.0

use aster_rights::{Read, TRightSet, TRights, Write};
use aster_rights_proc::require;

use super::*;
use crate::prelude::*;

impl<R: TRights> InodeHandle<TRightSet<R>> {
    #[require(R > Read)]
    pub fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        self.0.read(writer)
    }

    #[require(R > Read)]
    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        self.0.read_to_end(buf)
    }

    #[require(R > Write)]
    pub fn write(&self, reader: &mut VmReader) -> Result<usize> {
        self.0.write(reader)
    }

    #[require(R > Read)]
    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.0.readdir(visitor)
    }
}
