// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        ramfs::RamFS,
        utils::{InodeMode, InodeType},
    },
    prelude::*,
};

/// Initializes "/dev/shm" for POSIX shared memory usage.
pub fn init() -> Result<()> {
    let dev_path = {
        let fs = FsResolver::new();
        fs.lookup(&FsPath::try_from("/dev")?)?
    };

    // Create the "shm" directory under "/dev" and mount a ramfs on it.
    let shm_path =
        dev_path.new_fs_child("shm", InodeType::Dir, InodeMode::from_bits_truncate(0o1777))?;
    shm_path.mount(RamFS::new())?;
    log::debug!("Mount RamFS at \"/dev/shm\"");
    Ok(())
}
