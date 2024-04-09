// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        devpts::DevPts,
        fs_resolver::{FsPath, FsResolver},
        utils::{Inode, InodeMode, InodeType, Path},
    },
    prelude::*,
};

#[allow(clippy::module_inception)]
mod pty;

pub use pty::{PtyMaster, PtySlave};
use spin::Once;

static DEV_PTS: Once<Arc<Path>> = Once::new();

pub fn init() -> Result<()> {
    let fs = FsResolver::new();

    let dev = fs.lookup(&FsPath::try_from("/dev")?)?;
    // Create the "pts" directory and mount devpts on it.
    let devpts_dentry =
        dev.dentry()
            .create("pts", InodeType::Dir, InodeMode::from_bits_truncate(0o755))?;
    let devpts_mount_node =
        Path::new(dev.mount_node().clone(), devpts_dentry.clone()).mount(DevPts::new())?;
    let devpts = Path::new(
        devpts_mount_node.clone(),
        devpts_mount_node.root_dentry().clone(),
    );

    devpts.mount(DevPts::new())?;

    DEV_PTS.call_once(|| devpts);

    // Create the "ptmx" symlink.
    let ptmx = dev.dentry().create(
        "ptmx",
        InodeType::SymLink,
        InodeMode::from_bits_truncate(0o777),
    )?;
    ptmx.inode().write_link("pts/ptmx")?;
    Ok(())
}

pub fn new_pty_pair(index: u32, ptmx: Arc<dyn Inode>) -> Result<(Arc<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index);
    let slave = PtySlave::new(&master);
    Ok((master, slave))
}
