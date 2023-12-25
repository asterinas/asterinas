use crate::prelude::*;
use aster_rights::{Read, TRightSet, TRights, Write};
use aster_rights_proc::require;

use super::*;

impl<R: TRights> InodeHandle<TRightSet<R>> {
    #[require(R > Read)]
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }

    #[require(R > Read)]
    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        self.0.read_to_end(buf)
    }

    #[require(R > Write)]
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        self.0.write(buf)
    }

    #[require(R > Read)]
    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.0.readdir(visitor)
    }
}
