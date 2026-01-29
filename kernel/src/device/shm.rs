// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        path::{FsPath, PathResolver, PerMountFlags},
        ramfs::RamFs,
        utils::{InodeType, chmod},
    },
    prelude::*,
};

/// Initializes "/dev/shm" for POSIX shared memory usage.
pub fn init_in_first_process(path_resolver: &PathResolver, ctx: &Context) -> Result<()> {
    use crate::fs::utils::InodeMode;

    let dev_path = path_resolver.lookup(&FsPath::try_from("/dev")?)?;

    // Create the "shm" directory under "/dev" and mount a ramfs on it.
    let shm_path =
        dev_path.new_fs_child("shm", InodeType::Dir, chmod!(InodeMode::S_ISVTX, a+rwx))?;
    shm_path.mount(
        RamFs::new(),
        PerMountFlags::default(),
        Some("tmpfs".to_string()),
        ctx,
    )?;
    log::debug!("Mount RamFs at \"/dev/shm\"");
    Ok(())
}
