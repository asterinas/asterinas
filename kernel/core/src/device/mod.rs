// SPDX-License-Identifier: MPL-2.0

mod evdev;
mod fb;
mod mem;
pub(crate) mod misc;
mod model;
mod pty;
pub mod registry;
pub(crate) mod tty;

use device_id::{DeviceId, MajorIdOwner, MinorId};
pub(crate) use mem::{getrandom, geturandom};
pub(crate) use pty::{PtyMaster, PtySlave, new_pty_pair};
pub(crate) use registry::lookup;

use crate::{
    fs::{devtmpfs::DevtmpfsNodeMeta, file::PerOpenFileOps},
    prelude::*,
};

/// The abstraction of a device.
pub trait Device: Send + Sync + 'static {
    /// Returns the device type.
    fn type_(&self) -> DeviceType;

    /// Returns the owned major ID and the minor ID of the device.
    ///
    /// Every device must hold the ownership of its major ID via a [`MajorIdOwner`],
    /// ensuring that the major ID has been properly acquired from the device registry.
    fn owned_id(&self) -> (&MajorIdOwner, MinorId);

    /// Returns the metadata that specifies a device inode to be created in devtmpfs, if any.
    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta>;

    /// Opens the device, returning a file-like object that the userspace can interact with by
    /// doing I/O.
    fn open(&self) -> Result<Box<dyn PerOpenFileOps>>;
}

impl dyn Device {
    /// Returns the device ID.
    pub fn id(&self) -> DeviceId {
        let (major_owner, minor) = self.owned_id();
        DeviceId::new(major_owner.get(), minor)
    }
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

pub(crate) fn init_in_first_kthread() {
    model::init_in_first_kthread();
    registry::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    // TODO: Transfer ownership of the boot framebuffer to DRM and skip registering the
    // legacy framebuffer device once DRM has initialized successfully.
    fb::init_in_first_kthread();
}

/// Initializes device state after mounting rootfs.
pub(crate) fn init_in_first_process() -> Result<()> {
    tty::init_in_first_process()?;
    registry::init_in_first_process()?;

    Ok(())
}
