// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use ostd::arch::qemu::{exit_qemu, QemuExitCode};

use super::{KObject, SysFS, UEvent, SYSFS_REF};
use crate::{
    fs::kernfs::{DataProvider, PseudoExt},
    prelude::*,
    util::MultiRead,
};

/// Registers `/sys/power/state` in the SysFS.
pub(super) fn register_power_state() -> Result<()> {
    let power_kobj = SYSFS_REF
        .get()
        .ok_or(Errno::ENOENT)?
        .init_parent_dirs("/sys/power/")?;
    let _ = SysFS::create_attribute("state", 0o644, power_kobj, Box::new(State), None)?;
    Ok(())
}

/// Represents the power state attribute in sysfs
pub struct State;

impl DataProvider for State {
    fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
        let data = "freeze mem disk\n".as_bytes().to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, reader: &mut VmReader, _offset: usize) -> Result<usize> {
        let mut buffer = vec![0u8; reader.remain()];
        reader.read(&mut buffer.as_mut_slice().into())?;
        let buffer = core::str::from_utf8(&buffer)?;
        // FIXME: Implement the power state transition
        match buffer {
            "poweroff\n" => {
                exit_qemu(QemuExitCode::Success);
            }
            "reboot\n" => {
                todo!();
            }
            "mem\n" => {
                todo!();
            }
            "disk\n" => {
                todo!();
            }
            "freeze\n" => {
                todo!();
            }
            _ => {}
        }
        Ok(buffer.len())
    }

    fn truncate(&mut self, _new_size: usize) -> Result<()> {
        Ok(())
    }
}
