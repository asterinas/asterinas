// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        devpts::DevPts,
        fs_resolver::{FsPath, FsResolver},
        path::Path,
        utils::{Inode, InodeMode, InodeType},
    },
    prelude::*,
};

mod driver;
mod master;

pub use driver::PtySlave;
pub use master::PtyMaster;
use spin::Once;

static DEV_PTS: Once<Path> = Once::new();

pub fn init_in_first_process(fs_resolver: &FsResolver, ctx: &Context) -> Result<()> {
    let dev = fs_resolver.lookup(&FsPath::try_from("/dev")?)?;
    // Create the "pts" directory and mount devpts on it.
    let devpts_path =
        dev.new_fs_child("pts", InodeType::Dir, InodeMode::from_bits_truncate(0o755))?;
    let devpts_mount = devpts_path.mount(DevPts::new(), ctx)?;

    DEV_PTS.call_once(|| Path::new_fs_root(devpts_mount));

    // Create the "ptmx" symlink.
    let ptmx = dev.new_fs_child(
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
    let slave = master.slave().clone();
    Ok((master, slave))
}
