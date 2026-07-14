// SPDX-License-Identifier: MPL-2.0

mod evdev;
mod fb;
mod mem;
pub mod misc;
mod pty;
mod registry;
pub mod tty;

use device_id::DeviceId;
pub use mem::{getrandom, geturandom};
pub use pty::{PtyMaster, PtySlave, new_pty_pair};
pub use registry::lookup;

use crate::{
    fs::{devtmpfs::DevtmpfsInodeMeta, file::PerOpenFileOps},
    prelude::*,
};

/// The abstraction of a device.
pub trait Device: Send + Sync + 'static {
    /// Returns the device type.
    fn type_(&self) -> DeviceType;

    /// Returns the device ID.
    fn id(&self) -> DeviceId;

    /// Returns the metadata that specifies a device inode to be created in devtmpfs, if any.
    fn devtmpfs_meta(&self) -> Option<DevtmpfsInodeMeta<'static>> {
        None
    }

    /// Opens the device, returning a file-like object that the userspace can interact with by
    /// doing I/O.
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>>;
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.type_())
            .field("id", &self.id())
            .field("devtmpfs_meta", &self.devtmpfs_meta())
            .finish_non_exhaustive()
    }
}

/// Device type
#[derive(Debug)]
pub enum DeviceType {
    Char,
    Block,
}

pub fn init_in_first_kthread() {
    registry::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    fb::init_in_first_kthread();
    aster_block::init_in_first_kthread();
}

/// Initializes device state after mounting rootfs.
pub fn init_in_first_process() -> Result<()> {
    tty::init_in_first_process()?;
    registry::init_in_first_process()?;

    Ok(())
}
