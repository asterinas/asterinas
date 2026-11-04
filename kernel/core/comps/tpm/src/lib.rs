// SPDX-License-Identifier: MPL-2.0

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{ComponentInitError, init_component};
use spin::Once;

macro_rules! __log_prefix {
    () => {
        "tpm: "
    };
}

pub mod device;
pub mod extcrypto;
pub mod mmio;
pub mod space_io;

pub use device::{TPM_TIS_BASE, TPM_TIS_SIZE, TpmDevice, TpmInitErr, probe};
pub use extcrypto::{ExtAesCfb, ExtHmacSha256, ExtRng, ExtSha256, HASH_CTX_CAP, check_abi};
pub use mmio::TisMmio;
pub use space_io::{CcAttrs, CcTable, SPACE_BUF, XmitErr, space_transmit};
pub use tpm_core::{
    ChipLink,
    module::{IoErr, Space, load_space, save_space},
    rewrite::read_be32,
};

static DEVICE: Once<TpmDevice> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    #[cfg(target_arch = "x86_64")]
    match probe() {
        Ok(device) => {
            DEVICE.call_once(|| device);
        }
        Err(_error) => ostd::warn!("TPM initialization failed"),
    }
    Ok(())
}

pub fn device() -> Option<&'static TpmDevice> {
    DEVICE.get()
}
