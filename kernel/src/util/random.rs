// SPDX-License-Identifier: MPL-2.0

use rand::{rngs::StdRng, RngCore, SeedableRng};
use spin::Once;

use crate::prelude::*;

static RNG: Once<SpinLock<StdRng>> = Once::new();

/// Fill `dest` with random bytes.
///
/// It's cryptographically secure, as documented in [`rand::rngs::StdRng`].
pub fn getrandom(dst: &mut [u8]) {
    RNG.get().unwrap().lock().fill_bytes(dst);
}

pub fn init() {
    // The seed used to initialize the RNG is required to be secure and unpredictable.
    let seed = get_random_seed();

    RNG.call_once(|| SpinLock::new(StdRng::from_seed(seed)));
}

#[cfg(target_arch = "x86_64")]
fn get_random_seed() -> <StdRng as SeedableRng>::Seed {
    use ostd::arch::read_random;

    let mut seed = <StdRng as SeedableRng>::Seed::default();

    let mut chunks = seed.as_mut().chunks_exact_mut(size_of::<u64>());
    for chunk in chunks.by_ref() {
        let src = read_random().expect("`read_random` failed").to_ne_bytes();
        chunk.copy_from_slice(&src);
    }
    let tail = chunks.into_remainder();
    let n = tail.len();
    if n > 0 {
        let src = read_random().expect("`read_random` failed").to_ne_bytes();
        tail.copy_from_slice(&src[..n]);
    }

    seed
}

#[cfg(not(target_arch = "x86_64"))]
fn get_random_seed() -> <StdRng as SeedableRng>::Seed {
    use ostd::arch::boot::DEVICE_TREE;

    let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();
    let seed = chosen
        .property("rng-seed")
        .unwrap()
        .value
        .try_into()
        .unwrap();
    seed
}
