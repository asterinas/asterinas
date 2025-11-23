// SPDX-License-Identifier: MPL-2.0

mod char;
mod disk;
mod evdev;
mod fb;
mod mem;
pub mod misc;
mod pty;
mod shm;
pub mod tty;

use device_id::DeviceId;
pub use mem::{getrandom, geturandom};
pub use pty::{new_pty_pair, PtyMaster, PtySlave};

use crate::{
    fs::{device::Device, fs_resolver::FsPath, path::PerMountFlags, ramfs::RamFs},
    prelude::*,
};

pub fn init_in_first_kthread() {
    disk::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    fb::init_in_first_kthread();
}

/// Initializes the device nodes in devtmpfs after mounting rootfs.
pub fn init_in_first_process(ctx: &Context) -> Result<()> {
    let fs = ctx.thread_local.borrow_fs();
    let fs_resolver = fs.resolver().read();

    // Mount devtmpfs.
    let dev_path = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;
    dev_path.mount(RamFs::new(), PerMountFlags::default(), ctx)?;

    tty::init_in_first_process()?;
    pty::init_in_first_process(&fs_resolver, ctx)?;
    shm::init_in_first_process(&fs_resolver, ctx)?;
    char::init_in_first_process(&fs_resolver)?;
    disk::init_in_first_process(&fs_resolver)?;

    Ok(())
}

/// Looks up a device according to the device ID.
pub fn get_device(devid: DeviceId) -> Result<Arc<dyn Device>> {
    // TODO: Add support for looking up block devices.
    char::lookup(devid).ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "the device ID is invalid or unsupported")
    })
}
