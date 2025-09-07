// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        fs_resolver::{FsPath, FsResolver},
        ramfs::RamFs,
        utils::{InodeMode, InodeType},
    },
    prelude::*,
};

/// Initializes "/dev/shm" for POSIX shared memory usage.
pub fn init_in_first_process(fs_resolver: &FsResolver) -> Result<()> {
    let dev_path = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;

    // Create the "shm" directory under "/dev" and mount a ramfs on it.
    let shm_path =
        dev_path.new_fs_child("shm", InodeType::Dir, InodeMode::from_bits_truncate(0o1777))?;
    shm_path.mount(RamFs::new())?;
    log::debug!("Mount RamFs at \"/dev/shm\"");
    Ok(())
}
