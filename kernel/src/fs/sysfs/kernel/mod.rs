// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use super::{KObject, SysFS};
use crate::{fs::kernfs::DataProvider, prelude::*};

/// Initializes the kernel-related files in the SysFS.
pub fn register_kernel(root: Arc<KObject>) -> Result<()> {
    let kernel_kobj = SysFS::create_kobject("kernel", 0o755, root, None).unwrap();
    let mm_kobject = SysFS::create_kobject("mm", 0o755, kernel_kobj.clone(), None)?;
    let transparent_hugepage_kobject =
        SysFS::create_kobject("transparent_hugepage", 0o755, mm_kobject.clone(), None)?;
    SysFS::create_attribute(
        "hpage_pmd_size",
        0o444,
        transparent_hugepage_kobject.clone(),
        Box::new(HugepagePmdSize),
        None,
    )?;
    Ok(())
}

// Actually, we don't support transparent_hugepage now, so we just return 0.
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
