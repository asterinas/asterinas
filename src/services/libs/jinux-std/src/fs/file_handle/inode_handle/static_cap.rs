use crate::prelude::*;
use crate::rights::*;
use jinux_rights_proc::require;

use super::*;

impl<R: TRights> InodeHandle<R> {
    #[require(R > Read)]
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }

    #[require(R > Write)]
    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        self.0.write(buf)
    }

    #[require(R > Read)]
    pub fn readdir(&self, writer: &mut dyn DirentWriter) -> Result<usize> {
        self.0.readdir(writer)
    }
}
