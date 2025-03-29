// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use ostd::cpu::num_cpus;

use super::{KObject, SysFS, SYSFS_REF};
use crate::{fs::kernfs::DataProvider, prelude::*};

/// Registers `/sys/devices/system/cpu/online` in the SysFS.
pub(super) fn register_cpu_online() -> Result<()> {
    let cpu_kobject = SYSFS_REF
        .get()
        .ok_or(Errno::ENOENT)?
        .init_parent_dirs("/sys/devices/system/cpu/")?;
    let _ = SysFS::create_attribute("online", 0o444, cpu_kobject, Box::new(CpuOnline), None)?;
    Ok(())
}

/// The data provider for the CPU online file.
/// It returns the online CPUs in the format of "0-<num_cpus>\n".
/// It is read-only.
struct CpuOnline;

impl DataProvider for CpuOnline {
    fn read_at(&self, writer: &mut ostd::mm::VmWriter, offset: usize) -> Result<usize> {
        let data = format!("0-{}\n", num_cpus() - 1).as_bytes().to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, reader: &mut ostd::mm::VmReader, offset: usize) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }

    fn truncate(&mut self, _new_size: usize) -> Result<()> {
        return_errno!(Errno::EINVAL);
    }
}
