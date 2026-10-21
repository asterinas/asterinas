// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::file::{Mappable, MappedObject},
    prelude::*,
    vm::{
        page_cache::{Vmo, VmoMapMode, VmoOptions},
        vmar::MapHandle,
    },
};

pub(crate) struct KvmRunPage {
    run_page: Arc<Vmo>,
}

impl KvmRunPage {
    pub(crate) fn new(size: usize) -> Result<Self> {
        Ok(Self {
            run_page: VmoOptions::new(size).alloc()?,
        })
    }

    pub(super) fn read_run_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut value = T::new_zeroed();
        let mut writer = VmWriter::from(value.as_mut_bytes()).to_fallible();
        self.run_page.read(offset, &mut writer)?;
        Ok(value)
    }

    #[expect(dead_code)]
    pub(super) fn read_run_bytes(&self, offset: usize, buffer: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(buffer).to_fallible();
        self.run_page.read(offset, &mut writer)
    }

    pub(super) fn write_run_val<T: Pod>(&self, offset: usize, value: &T) -> Result<()> {
        let mut reader = VmReader::from(value.as_bytes()).to_fallible();
        self.run_page.write(offset, &mut reader)
    }

    pub(super) fn write_run_bytes(&self, offset: usize, buffer: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(buffer).to_fallible();
        self.run_page.write(offset, &mut reader)
    }
}

impl Mappable for KvmRunPage {
    fn map(&self, offset: usize, handle: MapHandle) -> Result<Box<dyn MappedObject>> {
        let size = self.run_page.size();
        if offset > size || handle.size() > size - offset {
            return_errno_with_message!(Errno::EINVAL, "mapping exceeds the run page");
        }

        Ok(Box::new(MappedRunPage {
            run_page: self.run_page.clone(),
            offset,
        }))
    }
}

#[derive(Debug)]
struct MappedRunPage {
    run_page: Arc<Vmo>,
    offset: usize,
}

impl MappedObject for MappedRunPage {
    fn dup_at_offset(&self, offset: usize) -> Box<dyn MappedObject> {
        Box::new(Self {
            run_page: self.run_page.clone(),
            offset: self.offset + offset,
        })
    }

    fn handle_page_fault(&self, offset: usize, mut handle: MapHandle) -> Result<()> {
        let page = loop {
            match self
                .run_page
                .try_commit_page(self.offset + offset, VmoMapMode::SharedWrite)
            {
                Ok((page, _)) => break page,
                Err(err) => self
                    .run_page
                    .commit_on(err.pending_index()?, VmoMapMode::SharedWrite)?,
            }
        };
        handle.map_frame(offset, page.into());
        Ok(())
    }
}
