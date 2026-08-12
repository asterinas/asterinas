// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        devpts::{DevPts, Ptmx},
        file::{InodeType, mkmod},
        vfs::{
            path::{FsPath, PathResolver, PerMountFlags},
            registry::FsAndRoot,
        },
    },
    prelude::*,
};

mod driver;
mod file;
mod ioctl_defs;
mod master;
mod packet;

pub(crate) use driver::PtySlave;
pub(crate) use master::PtyMaster;

pub(crate) fn init_in_first_process(path_resolver: &PathResolver, ctx: &Context) -> Result<()> {
    let dev = path_resolver.lookup(&FsPath::try_from("/dev")?)?;

    // Create the "pts" directory and mount devpts on it.
    let devpts_path = dev.new_fs_child("pts", InodeType::Dir, mkmod!(a+rx, u+w))?;
    devpts_path.mount(
        FsAndRoot::new(DevPts::new()),
        PerMountFlags::default(),
        Some("devpts".to_string()),
        ctx,
    )?;

    // Create the "ptmx" symlink.
    let ptmx = dev.new_fs_child("ptmx", InodeType::SymLink, mkmod!(a+rwx))?;
    ptmx.inode().write_link("pts/ptmx")?;

    Ok(())
}

pub(crate) fn new_pty_pair(index: u32, ptmx: Arc<Ptmx>) -> Result<(Box<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index);
    let slave = master.slave().clone();
    Ok((master, slave))
}
