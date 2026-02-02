// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        devpts::{DevPts, Ptmx},
        path::{FsPath, Path, PathResolver, PerMountFlags},
        utils::{InodeType, mkmod},
    },
    prelude::*,
};

mod driver;
mod file;
mod master;
mod packet;

pub use driver::PtySlave;
pub use master::PtyMaster;
use spin::Once;

static DEV_PTS: Once<Path> = Once::new();

pub fn init_in_first_process(path_resolver: &PathResolver, ctx: &Context) -> Result<()> {
    let dev = path_resolver.lookup(&FsPath::try_from("/dev")?)?;
    // Create the "pts" directory and mount devpts on it.
    let devpts_path = dev.new_fs_child("pts", InodeType::Dir, mkmod!(a+rx, u+w))?;
    let devpts_mount = devpts_path.mount(
        DevPts::new(),
        PerMountFlags::default(),
        Some("devpts".to_string()),
        ctx,
    )?;

    DEV_PTS.call_once(|| Path::new_fs_root(devpts_mount));

    // Create the "ptmx" symlink.
    let ptmx = dev.new_fs_child("ptmx", InodeType::SymLink, mkmod!(a+rwx))?;
    ptmx.inode().write_link("pts/ptmx")?;
    Ok(())
}

pub fn new_pty_pair(index: u32, ptmx: Arc<Ptmx>) -> Result<(Box<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index);
    let slave = master.slave().clone();
    Ok((master, slave))
}
