// SPDX-License-Identifier: MPL-2.0

use crate::{fs::devpts::Ptmx, prelude::*};

mod driver;
mod file;
mod ioctl_defs;
mod master;
mod packet;

pub(crate) use driver::PtySlave;
pub(crate) use master::PtyMaster;

pub(crate) fn new_pty_pair(index: u32, ptmx: Arc<Ptmx>) -> Result<(Box<PtyMaster>, Arc<PtySlave>)> {
    debug!("pty index = {}", index);
    let master = PtyMaster::new(ptmx, index);
    let slave = master.slave().clone();
    Ok((master, slave))
}
