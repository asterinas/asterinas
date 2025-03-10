// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, format};

use super::Tty;
use crate::{
    fs::{
        device::{Device, DeviceId},
        kernfs::DataProvider,
        sysfs::{SysFS, SYSFS_REF},
    },
    prelude::*,
};

/// Register the tty device in the SysFS.
pub(super) fn register_tty(tty_device: Arc<Tty>) -> Result<()> {
    let tty_subsystem = SYSFS_REF
        .get()
        .ok_or(Errno::ENOENT)?
        .init_parent_dirs("/sys/devices/virtual/tty/")?;
    let tty_template = SysFS::create_dir(&tty_device.name(), 0o755, tty_subsystem, None)?;
    let _ = SysFS::create_attribute(
        "dev",
        0o444,
        tty_template.clone(),
        Box::new(TtyDevID::new(tty_device.id())),
        None,
    )?;
    let _ = SysFS::create_attribute(
        "uevent",
        0o644,
        tty_template.clone(),
        Box::new(TtyUEvent::new(tty_device.id(), tty_device.name())),
        None,
    )?;
    // Exports `/sys/class/tty/<tty>/` as a symlink to `/sys/devices/virtual/tty/<tty>/`
    let tty_class = SYSFS_REF
        .get()
        .ok_or(Errno::ENOENT)?
        .init_parent_dirs("/sys/class/tty/")?;
    let _ = SysFS::create_symlink(
        &tty_device.name(),
        tty_class,
        &format!("../../devices/virtual/tty/{}", tty_device.name()),
        None,
    )?;
    Ok(())
}

/// Represents `/sys/class/tty/<tty>/dev`, e.g., `/sys/class/tty/tty0/dev`
struct TtyDevID {
    id: DeviceId,
}

impl TtyDevID {
    pub fn new(id: DeviceId) -> Self {
        Self { id }
    }
}

impl DataProvider for TtyDevID {
    fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
        let data = format!("{}:{}\n", self.id.major(), self.id.minor())
            .as_bytes()
            .to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, _reader: &mut VmReader, _offset: usize) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }

    fn truncate(&mut self, _new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }
}

/// Represents `/sys/class/tty/<tty>/uevent`, e.g., `/sys/class/tty/tty0/uevent`
struct TtyUEvent {
    id: DeviceId,
    name: String,
}

impl TtyUEvent {
    pub fn new(id: DeviceId, name: String) -> Self {
        Self { id, name }
    }
}

impl DataProvider for TtyUEvent {
    fn read_at(&self, writer: &mut VmWriter, offset: usize) -> Result<usize> {
        let data = format!(
            "MAJOR={}\nMINOR={}\nDEVNAME={}\n",
            self.id.major(),
            self.id.minor(),
            self.name
        )
        .as_bytes()
        .to_vec();
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn write_at(&mut self, _reader: &mut VmReader, _offset: usize) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }

    fn truncate(&mut self, _new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "This file is read-only");
    }
}
