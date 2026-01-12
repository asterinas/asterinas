// SPDX-License-Identifier: MPL-2.0

mod evdev;
mod fb;
mod mem;
pub mod misc;
mod pty;
mod registry;
mod shm;
pub mod tty;

pub use mem::{getrandom, geturandom};
pub use pty::{PtyMaster, PtySlave, new_pty_pair};
pub use registry::lookup;

use crate::{
    fs::{
        path::{FsPath, PerMountFlags},
        ramfs::RamFs,
    },
    prelude::*,
};

pub fn init_in_first_kthread() {
    registry::init_in_first_kthread();
    mem::init_in_first_kthread();
    misc::init_in_first_kthread();
    evdev::init_in_first_kthread();
    fb::init_in_first_kthread();
}

/// Initializes the device nodes in devtmpfs after mounting rootfs.
pub fn init_in_first_process(ctx: &Context) -> Result<()> {
    let fs = ctx.thread_local.borrow_fs();
    let path_resolver = fs.resolver().read();

    // Mount devtmpfs.
    let dev_path = path_resolver.lookup(&FsPath::try_from("/dev")?)?;
    dev_path.mount(RamFs::new(), PerMountFlags::default(), ctx)?;

    tty::init_in_first_process()?;
    pty::init_in_first_process(&path_resolver, ctx)?;
    shm::init_in_first_process(&path_resolver, ctx)?;
    registry::init_in_first_process(&path_resolver)?;

    Ok(())
}
