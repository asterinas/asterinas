// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use rand::{rngs::StdRng, Error as RandError, RngCore};
use spin::Once;

use crate::prelude::*;

static RNG: Once<SpinLock<StdRng>> = Once::new();

/// Fill `dest` with random bytes.
///
/// It's cryptographically secure, as documented in [`rand::rngs::StdRng`].
pub fn getrandom(dst: &mut [u8]) -> Result<()> {
    Ok(RNG.get().unwrap().lock().try_fill_bytes(dst)?)
}

pub fn init() {
    // The seed used to initialize the RNG is required to be secure and unpredictable.

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            use rand::SeedableRng;

            RNG.call_once(|| SpinLock::new(StdRng::from_entropy()));
        } else {
            compile_error!("unsupported target");
        }
    }
}

impl From<RandError> for Error {
    fn from(value: RandError) -> Self {
        Error::with_message(Errno::ENOSYS, "cannot generate random bytes")
    }
}
