mod null;
pub mod tty;
mod zero;

use crate::fs::device::{add_node, Device, DeviceId, DeviceType};
use crate::prelude::*;

/// Init the device node in fs, must be called after mounting rootfs.
pub fn init() -> Result<()> {
    let null = Arc::new(null::Null);
    add_node(null, "null")?;
    let zero = Arc::new(zero::Zero);
    add_node(zero, "zero")?;
    tty::init();
    let tty = tty::get_n_tty().clone();
    add_node(tty, "tty")?;
    Ok(())
}
