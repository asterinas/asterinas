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
    fs::{
        device::{add_node, Device},
        fs_resolver::FsPath,
        path::PerMountFlags,
        ramfs::RamFs,
    },
    prelude::*,
};

pub fn init_in_first_kthread() {
    disk::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    fb::init_in_first_kthread();
}

/// Init the device node in fs, must be called after mounting rootfs.
pub fn init_in_first_process(ctx: &Context) -> Result<()> {
    let fs = ctx.thread_local.borrow_fs();
    let fs_resolver = fs.resolver().read();

    // Mount DevFS
    let dev_path = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;
    dev_path.mount(RamFs::new(), PerMountFlags::default(), ctx)?;

    tty::init_in_first_process();

    let tty0 = Arc::new(tty::Tty0Device);
    add_node(tty0, "tty0", &fs_resolver)?;

    let tty1 = tty::tty1_device().clone();
    add_node(tty1, "tty1", &fs_resolver)?;

    let tty = Arc::new(tty::TtyDevice);
    add_node(tty, "tty", &fs_resolver)?;

    let console = tty::SystemConsole::singleton().clone();
    add_node(console, "console", &fs_resolver)?;

    if let Some(hvc0) = tty::hvc0_device() {
        add_node(hvc0.clone(), "hvc0", &fs_resolver)?;
    }

    pty::init_in_first_process(&fs_resolver, ctx)?;

    shm::init_in_first_process(&fs_resolver, ctx)?;

    char::init_in_first_process(&fs_resolver)?;

    disk::init_in_first_process(&fs_resolver)?;

    Ok(())
}

// TODO: Implement a more scalable solution for ID-to-device mapping.
// Instead of hardcoding every device numbers in this function,
// a registration mechanism should be used to allow each driver to
// allocate device IDs either statically or dynamically.
pub fn get_device(devid: DeviceId) -> Result<Arc<dyn Device>> {
    let major = devid.major().get();
    let minor = devid.minor().get();

    match (major, minor) {
        (4, 0) => Ok(Arc::new(tty::Tty0Device)),
        (4, 1) => Ok(tty::tty1_device().clone()),
        (5, 0) => Ok(Arc::new(tty::TtyDevice)),
        (5, 1) => Ok(tty::SystemConsole::singleton().clone()),
        (229, 0) => tty::hvc0_device()
            .cloned()
            .map(|device| device as _)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "the hvc0 device is not available")),
        _ => char::lookup(devid)
            .map(|device| Arc::new(char::CharFile::new(device)) as Arc<dyn Device>)
            .ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "the device ID is invalid or unsupported")
            }),
    }
}
