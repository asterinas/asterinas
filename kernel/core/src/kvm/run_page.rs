// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    vm::page_cache::{Vmo, VmoOptions},
    fs::{
        file::{MappableObject},
    },
};

pub struct KvmRun {
    run_page: Arc<Vmo>,
}

impl KvmRun {
    pub fn new(size: usize) -> Result<Self> {
        Ok(Self {
            run_page: VmoOptions::new(size).alloc()?
        })
    }

    pub fn mappable(&self) -> Result<MappableObject<'_>> {
        Ok(MappableObject::Vmo(self.run_page.clone()))
    }

    pub fn read_run_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut value = T::new_zeroed();
        let mut writer = VmWriter::from(value.as_mut_bytes()).to_fallible();
        self.run_page.read(offset, &mut writer)?;
        Ok(value)
    }

    pub fn write_run_val<T: Pod>(&self, offset: usize, value: &T) -> Result<()> {
        let mut reader = VmReader::from(value.as_bytes()).to_fallible();
        self.run_page.write(offset, &mut reader)
    }

    pub fn read_run_bytes(&self, offset: usize, buffer: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(buffer).to_fallible();
        self.run_page.read(offset, &mut writer)
    }

    pub fn write_run_bytes(&self, offset: usize, buffer: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(buffer).to_fallible();
        self.run_page.write(offset, &mut reader)
    }
}
