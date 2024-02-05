// SPDX-License-Identifier: MPL-2.0

mod null;
mod pty;
mod random;
#[cfg(feature = "intel_tdx")]
mod tdxguest;
pub mod tty;
mod urandom;
mod zero;

use crate::fs::device::{add_node, Device, DeviceId, DeviceType};
use crate::prelude::*;
pub use pty::new_pty_pair;
pub use pty::{PtyMaster, PtySlave};
pub use random::Random;
#[cfg(feature = "intel_tdx")]
pub use tdxguest::TdxGuest;
pub use urandom::Urandom;

use self::tty::get_n_tty;

/// Init the device node in fs, must be called after mounting rootfs.
pub fn init() -> Result<()> {
    let null = Arc::new(null::Null);
    add_node(null, "null")?;
    let zero = Arc::new(zero::Zero);
    add_node(zero, "zero")?;
    tty::init();
    let console = get_n_tty().clone();
    add_node(console, "console")?;
    let tty = Arc::new(tty::TtyDevice);
    add_node(tty, "tty")?;
    #[cfg(feature = "intel_tdx")]
    let tdx_guest = Arc::new(tdxguest::TdxGuest);
    #[cfg(feature = "intel_tdx")]
    add_node(tdx_guest, "tdx-guest")?;
    let random = Arc::new(random::Random);
    add_node(random, "random")?;
    let urandom = Arc::new(urandom::Urandom);
    add_node(urandom, "urandom")?;
    pty::init()?;
    Ok(())
}
