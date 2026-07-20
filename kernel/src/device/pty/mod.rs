// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{devpts::Ptmx, vfs::path::Path},
    prelude::*,
};

mod driver;
mod file;
mod ioctl_defs;
mod master;
mod packet;

pub use driver::PtySlave;
pub use master::PtyMaster;

pub fn new_pty_pair(
    index: u32,
    ptmx: Arc<Ptmx>,
    devpts_root: Path,
) -> Result<(Box<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index, devpts_root);
    let slave = master.slave().clone();
    Ok((master, slave))
}
