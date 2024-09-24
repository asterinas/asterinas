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
            use ostd::arch::read_random;

            let mut seed = <StdRng as SeedableRng>::Seed::default();
            let mut chunks = seed.as_mut().chunks_exact_mut(size_of::<u64>());
            for chunk in chunks.by_ref() {
                let src = read_random().expect("read_random failed multiple times").to_ne_bytes();
                chunk.copy_from_slice(&src);
            }
            let tail = chunks.into_remainder();
            let n = tail.len();
            if n > 0 {
                let src = read_random().expect("read_random failed multiple times").to_ne_bytes();
                tail.copy_from_slice(&src[..n]);
            }

            RNG.call_once(|| SpinLock::new(StdRng::from_seed(seed)));
        } else if #[cfg(target_arch = "riscv64")] {
            use rand::SeedableRng;
            use ostd::arch::boot::DEVICE_TREE;

            let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();
            let seed = chosen.property("rng-seed").unwrap().value.try_into().unwrap();

            RNG.call_once(|| SpinLock::new(StdRng::from_seed(seed)));
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
