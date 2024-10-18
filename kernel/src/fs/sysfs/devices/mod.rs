// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::cpu::num_cpus;

use super::{inode::KObject, SysFS};
use crate::{fs::kernfs::DataProvider, prelude::*};

/// Initializes the devices in the SysFS.
pub fn init_devices(devices_kobj: Arc<KObject>) -> Result<()> {
    let system_kobject = SysFS::create_kobject("system", 0o755, devices_kobj.clone())?;
    let cpu_kobject = SysFS::create_kobject("cpu", 0o755, system_kobject.clone())?;
    SysFS::create_file("online", 0o444, cpu_kobject.clone(), Box::new(CpuOnline))?;
    Ok(())
}

/// The data provider for the CPU online file.
/// It returns the online CPUs in the format of "0-<num_cpus>\n".
/// It is read-only.
struct CpuOnline;

impl DataProvider for CpuOnline {
    fn read_at(&self, writer: &mut ostd::mm::VmWriter, offset: usize) -> Result<usize> {
        let data = format!("0-{}\n", num_cpus()).as_bytes().to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, reader: &mut ostd::mm::VmReader, offset: usize) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }
}
