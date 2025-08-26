// SPDX-License-Identifier: MPL-2.0

mod null;
mod pty;
mod random;
mod shm;
pub mod tty;
mod urandom;
mod zero;

#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
mod tdxguest;

use alloc::format;

pub use pty::{new_pty_pair, PtyMaster, PtySlave};
pub use random::Random;
pub use urandom::Urandom;

use crate::{
    fs::{
        device::{add_node, Device, DeviceId, DeviceType},
        fs_resolver::FsPath,
        ramfs::RamFS,
    },
    prelude::*,
};

/// Init the device node in fs, must be called after mounting rootfs.
pub fn init_in_first_process(ctx: &Context) -> Result<()> {
    let fs = ctx.thread_local.borrow_fs();
    let fs_resolver = fs.resolver().read();

    // Mount DevFS
    let dev_path = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;
    dev_path.mount(RamFS::new(), ctx)?;

    let null = Arc::new(null::Null);
    add_node(null, "null", &fs_resolver)?;

    let zero = Arc::new(zero::Zero);
    add_node(zero, "zero", &fs_resolver)?;

    tty::init();

    let tty = Arc::new(tty::TtyDevice);
    add_node(tty, "tty", &fs_resolver)?;

    let console = tty::system_console().clone();
    add_node(console, "console", &fs_resolver)?;

    for (index, tty) in tty::iter_n_tty().enumerate() {
        add_node(tty.clone(), &format!("tty{}", index), &fs_resolver)?;
    }

    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        add_node(Arc::new(tdxguest::TdxGuest), "tdx_guest", &fs_resolver)?;
    });

    let random = Arc::new(random::Random);
    add_node(random, "random", &fs_resolver)?;

    let urandom = Arc::new(urandom::Urandom);
    add_node(urandom, "urandom", &fs_resolver)?;

    pty::init_in_first_process(&fs_resolver, ctx)?;

    shm::init_in_first_process(&fs_resolver, ctx)?;

    Ok(())
}

// TODO: Implement a more scalable solution for ID-to-device mapping.
// Instead of hardcoding every device numbers in this function,
// a registration mechanism should be used to allow each driver to
// allocate device IDs either statically or dynamically.
pub fn get_device(devid: DeviceId) -> Result<Arc<dyn Device>> {
    let major = devid.major();
    let minor = devid.minor();

    match (major, minor) {
        (1, 3) => Ok(Arc::new(null::Null)),
        (1, 5) => Ok(Arc::new(zero::Zero)),
        (5, 0) => Ok(Arc::new(tty::TtyDevice)),
        (1, 8) => Ok(Arc::new(random::Random)),
        (1, 9) => Ok(Arc::new(urandom::Urandom)),
        _ => return_errno_with_message!(Errno::EINVAL, "the device ID is invalid or unsupported"),
    }
}
