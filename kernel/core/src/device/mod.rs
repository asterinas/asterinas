// SPDX-License-Identifier: MPL-2.0

mod evdev;
mod fb;
mod mem;
pub(crate) mod misc;
mod model;
mod pty;
pub mod registry;
pub(crate) mod tty;

use aster_device::class::{Class, ClassDevice};
use device_id::DeviceId;
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

    /// Returns the device ID.
    fn id(&self) -> DeviceId;

    /// Returns the metadata that specifies a device inode to be created in devtmpfs, if any.
    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta>;

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

/// Adds a class device to the device model and the char registry.
///
/// If the char registry refuses the device,
/// the device model registration is undone rather than left dangling.
pub(crate) fn add_class_char_device<C: Class>(device: &Arc<ClassDevice<C>>) -> Result<()>
where
    ClassDevice<C>: Device,
{
    aster_device::add_device(device)?;
    if let Err(error) = registry::char::register(device.clone()) {
        let _ = aster_device::remove_device(device);
        return Err(error);
    }
    Ok(())
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
