mod master;
mod slave;

pub use master::PtyMaster;
pub use slave::PtySlave;

use crate::fs::{
    devpts::DevPts,
    fs_resolver::{FsPath, FsResolver},
    utils::{InodeMode, InodeType},
};
use crate::prelude::*;

pub fn init() -> Result<()> {
    let fs = FsResolver::new();

    let dev = fs.lookup(&FsPath::try_from("/dev")?)?;
    // Create the "pts" directory and mount devpts on it.
    let devpts = dev.create("pts", InodeType::Dir, InodeMode::from_bits_truncate(0o755))?;
    devpts.mount(DevPts::new())?;

    // Create the "ptmx" symlink.
    let ptmx = dev.create(
        "ptmx",
        InodeType::SymLink,
        InodeMode::from_bits_truncate(0o777),
    )?;
    ptmx.write_link("pts/ptmx")?;
    Ok(())
}
