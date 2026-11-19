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

mod device;
mod extcrypto;
mod mmio;
mod random;
mod space_io;

pub use device::TpmDevice;
pub use space_io::{SPACE_BUF, XmitErr};
pub use tpm_core::{
    TPM_HEADER_SIZE,
    module::{IoErr, Space},
    types::TPM_BUFSIZE,
};

static DEVICE: Once<TpmDevice> = Once::new();

#[init_component(kthread)]
fn init() -> Result<(), ComponentInitError> {
    if cfg!(target_arch = "x86_64") {
        match device::probe() {
            Ok(device) => {
                DEVICE.call_once(|| device);
            }
            Err(_error) => ostd::warn!("TPM initialization failed"),
        }
    }
    Ok(())
}

pub fn device() -> Option<&'static TpmDevice> {
    DEVICE.get()
}
