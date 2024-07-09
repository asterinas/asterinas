// SPDX-License-Identifier: MPL-2.0

use cfg_if::cfg_if;

mod null;
mod pty;
mod random;
pub mod tty;
mod urandom;
mod virtio_blk;
mod zero;

cfg_if! {
    if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
        mod tdxguest;

        use tdx_guest::tdx_is_enabled;

        pub use tdxguest::TdxGuest;
    }
}

pub use pty::{new_pty_pair, PtyMaster, PtySlave};
pub use random::Random;
pub use urandom::Urandom;
pub use virtio_blk::start_block_device;

use self::tty::get_n_tty;
use crate::{
    fs::device::{add_node, Device, DeviceId, DeviceType},
    prelude::*,
};

/// Init the device node in fs, must be called after mounting rootfs.
pub fn init() -> Result<()> {
    let null = Arc::new(null::Null);
    add_node(null, "null")?;
    let zero = Arc::new(zero::Zero);
    add_node(zero, "zero")?;
    tty::init();
    let console = get_n_tty().clone();
    add_node(console, "console")?;
    let tty = Arc::new(tty::TtyDevice);
    add_node(tty, "tty")?;
    cfg_if! {
        if #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))] {
            let tdx_guest = Arc::new(tdxguest::TdxGuest);

            if tdx_is_enabled() {
                add_node(tdx_guest, "tdx_guest")?;
            }
        }
    }
    let random = Arc::new(random::Random);
    add_node(random, "random")?;
    let urandom = Arc::new(urandom::Urandom);
    add_node(urandom, "urandom")?;
    pty::init()?;
    Ok(())
}

/// Lazily init the virtio-blk device node.
pub fn lazy_init() {
    // The device name is specified in qemu args as `--serial={device_name}`
    const VIRTIO_BLK_DEVICE_NAME: &str = "vda";
    let _ = start_block_device(VIRTIO_BLK_DEVICE_NAME).unwrap();

    let virtio_blk = Arc::new(virtio_blk::VirtioBlk::get_device(VIRTIO_BLK_DEVICE_NAME));
    let _ = add_node(virtio_blk, VIRTIO_BLK_DEVICE_NAME).unwrap();
}

// TODO: Implement a more scalable solution for ID-to-device mapping.
// Instead of hardcoding every device numbers in this function,
// a registration mechanism should be used to allow each driver to
// allocate device IDs either statically or dynamically.
pub fn get_device(dev: usize) -> Result<Arc<dyn Device>> {
    if dev == 0 {
        return_errno_with_message!(Errno::EPERM, "whiteout device")
    }

    let devid = DeviceId::from(dev as u64);
    let major = devid.major();
    let minor = devid.minor();

    match (major, minor) {
        (1, 3) => Ok(Arc::new(null::Null)),
        (1, 5) => Ok(Arc::new(zero::Zero)),
        (5, 0) => Ok(Arc::new(tty::TtyDevice)),
        (1, 8) => Ok(Arc::new(random::Random)),
        (1, 9) => Ok(Arc::new(urandom::Urandom)),
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported device"),
    }
}
