// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use super::{KObject, SysFS, SYSFS_REF};
use crate::{fs::kernfs::DataProvider, prelude::*};

/// Registers `/sys/kernel/mm/transparent_hugepage/hpage_pmd_size` in the SysFS.
pub(super) fn register_huge_page() -> Result<()> {
    let transparent_hugepage_kobject = SYSFS_REF
        .get()
        .ok_or(Errno::ENOENT)?
        .init_parent_dirs("/sys/kernel/mm/transparent_hugepage")?;
    let _ = SysFS::create_attribute(
        "hpage_pmd_size",
        0o444,
        transparent_hugepage_kobject,
        Box::new(HugepagePmdSize),
        None,
    )?;
    Ok(())
}

// FIXME: we don't support transparent_hugepage now, so we just return 0.
pub struct HugepagePmdSize;

impl DataProvider for HugepagePmdSize {
    fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
        let data = "0\n".as_bytes().to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, _reader: &mut VmReader, _offset: usize) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "HugepagePmdSize is read-only");
    }

    fn truncate(&mut self, _new_size: usize) -> Result<()> {
        return_errno!(Errno::EINVAL);
    }
}
