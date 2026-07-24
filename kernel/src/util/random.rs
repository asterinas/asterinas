// SPDX-License-Identifier: MPL-2.0

use blake2s::{BLAKE2S_HASH_SIZE, Blake2s};
use ostd::sync::WaitQueue;
use rand::{RngCore, SeedableRng, rngs::StdRng};
use spin::Once;

use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollee},
};

const READY_BITS: usize = BLAKE2S_HASH_SIZE * 8;

static RANDOM: Once<RandomState> = Once::new();

struct RandomState {
    inner: SpinLock<RandomInner>,
    pollee: Pollee,
    wait_queue: WaitQueue,
}

struct RandomInner {
    crng: StdRng,
    pool: InputPool,
    ready: bool,
}

struct InputPool {
    context: Blake2s,
    credited_bits: usize,
    extract_count: u64,
}

impl InputPool {
    fn new() -> Self {
        Self {
            context: Blake2s::new(),
            credited_bits: 0,
            extract_count: 0,
        }
    }

    fn mix(&mut self, bytes: &[u8]) {
        let len = bytes.len().to_ne_bytes();

        self.context.update(b"mix");
        self.context.update(&len);
        self.context.update(bytes);
    }

    fn credit(&mut self, bits: usize) {
        self.credited_bits = self.credited_bits.saturating_add(bits);
    }

    fn credited_bits(&self) -> usize {
        self.credited_bits
    }

    fn extract_seed(&mut self) -> <StdRng as SeedableRng>::Seed {
        let extract_count = self.extract_count.to_ne_bytes();
        self.extract_count = self.extract_count.saturating_add(1);

        let old_context = core::mem::take(&mut self.context);
        let digest = {
            let mut context = old_context;
            context.update(b"extract");
            context.update(&extract_count);
            context.finalize()
        };

        self.context.update(b"rekey");
        self.context.update(&digest);

        seed_from_hash(digest)
    }
}

impl RandomInner {
    fn new() -> Self {
        let mut inner = Self {
            crng: StdRng::from_seed(read_seed_from_timestamp()),
            pool: InputPool::new(),
            ready: false,
        };

        collect_boot_randomness(&mut inner);
        inner.try_initialize_crng();
        inner
    }

    fn mix_entropy(&mut self, bytes: &[u8]) {
        self.pool.mix(bytes);
    }

    fn mix_and_credit_entropy(&mut self, bytes: &[u8], credit_bits: usize) -> bool {
        self.pool.mix(bytes);
        self.pool.credit(credit_bits);
        self.try_initialize_crng()
    }

    fn try_initialize_crng(&mut self) -> bool {
        if self.ready || self.pool.credited_bits() < READY_BITS {
            return false;
        }

        let seed = self.pool.extract_seed();
        self.crng = StdRng::from_seed(seed);
        self.ready = true;
        true
    }
}

/// Fills `dest` with random bytes.
///
/// The returned bytes may be based on an insecure early seed if the random
/// subsystem has not been securely initialized yet.
pub fn getrandom(dst: &mut [u8]) {
    fill_insecure(dst);
}

/// Fills `dst` with best-effort random bytes without waiting for readiness.
pub fn fill_insecure(dst: &mut [u8]) {
    RANDOM.get().unwrap().inner.lock().crng.fill_bytes(dst);
}

/// Returns whether the random subsystem has been securely initialized.
pub fn is_ready() -> bool {
    RANDOM.get().unwrap().inner.lock().ready
}

/// Polls the readiness of the secure random stream.
pub fn poll(mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
    RANDOM
        .get()
        .unwrap()
        .pollee
        .poll_with(mask, poller, random_io_events)
}

/// Waits until the random subsystem has been securely initialized.
pub fn wait_until_ready() -> Result<()> {
    if is_ready() {
        return Ok(());
    }

    RANDOM
        .get()
        .unwrap()
        .wait_queue
        .pause_until(|| is_ready().then_some(()))
}

/// Returns `EAGAIN` if the random subsystem has not been securely initialized.
pub fn try_wait_until_ready() -> Result<()> {
    if is_ready() {
        return Ok(());
    }

    return_errno_with_message!(Errno::EAGAIN, "secure random data is not ready");
}

// TODO: Reseed the CRNG after readiness. Currently, entropy added after the
// random subsystem is ready is mixed into the input pool, but it does not
// affect subsequent `crng` output.
pub fn add_entropy(bytes: &[u8], credit_bits: usize) {
    let random = RANDOM.get().unwrap();
    let became_ready = random
        .inner
        .lock()
        .mix_and_credit_entropy(bytes, credit_bits);

    if became_ready {
        random.wait_queue.wake_all();
        random.pollee.notify(IoEvents::IN);
    }
}

pub fn init() {
    RANDOM.call_once(|| RandomState {
        inner: SpinLock::new(RandomInner::new()),
        pollee: Pollee::new(),
        wait_queue: WaitQueue::new(),
    });

    if RANDOM.get().unwrap().inner.lock().ready {
        RANDOM.get().unwrap().wait_queue.wake_all();
        RANDOM.get().unwrap().pollee.notify(IoEvents::IN);
    }
}

fn random_io_events() -> IoEvents {
    if is_ready() {
        IoEvents::IN | IoEvents::OUT
    } else {
        IoEvents::OUT
    }
}

fn collect_boot_randomness(inner: &mut RandomInner) {
    // Prefer the hardware RNG if available.
    if let Some(mut seed) = read_seed_from_hardware() {
        ostd::info!("use randomness generated by hardware");
        inner.mix_and_credit_entropy(seed.as_mut(), READY_BITS);
        return;
    }

    // x86_64: if no hardware RNG and running inside TDX, abort.
    //
    // For more details, see
    // <https://intel.github.io/ccc-linux-guest-hardening-docs/security-spec.html#randomness-inside-tdx-guest>.
    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({
        panic!("hardware randomness is mandatory for TD guests");
    });

    // Non-x86_64: fall back to the device-tree seed if present.
    #[cfg(not(target_arch = "x86_64"))]
    if let Some(mut seed) = read_seed_from_device_tree() {
        ostd::info!("use randomness provided by the device tree");
        inner.mix_and_credit_entropy(seed.as_mut(), READY_BITS);
        return;
    }

    // Some platforms (e.g., RISC-V `sifive_u`) have neither hardware
    // randomness nor randomness from the device tree. Continue with an
    // insecure early seed, but do not mark the random subsystem as ready.
    ostd::warn!("use randomness based on the timestamp, which is insecure");
    let mut seed = read_seed_from_timestamp();
    inner.mix_entropy(seed.as_mut());
}

fn read_seed_from_hardware() -> Option<<StdRng as SeedableRng>::Seed> {
    use ostd::arch::read_random;

    read_seed_from(read_random)
}

#[cfg(not(target_arch = "x86_64"))]
fn read_seed_from_device_tree() -> Option<<StdRng as SeedableRng>::Seed> {
    use ostd::arch::boot::DEVICE_TREE;

    DEVICE_TREE
        .get()
        .unwrap()
        .find_node("/chosen")
        .and_then(|chosen| chosen.property("rng-seed"))
        .and_then(|rng_seed| <StdRng as SeedableRng>::Seed::try_from(rng_seed.value).ok())
}

fn read_seed_from_timestamp() -> <StdRng as SeedableRng>::Seed {
    use ostd::arch::read_tsc;

    read_seed_from(|| Some(read_tsc())).unwrap()
}

fn read_seed_from(
    mut next_random: impl FnMut() -> Option<u64>,
) -> Option<<StdRng as SeedableRng>::Seed> {
    let mut seed = <StdRng as SeedableRng>::Seed::default();

    let (chunks, tail) = seed.as_mut().as_chunks_mut::<{ size_of::<u64>() }>();
    for chunk in chunks {
        let val = next_random()?;
        chunk.copy_from_slice(&val.to_ne_bytes());
    }
    if !tail.is_empty() {
        let val = next_random()?;
        tail.copy_from_slice(&val.to_ne_bytes()[..tail.len()]);
    }

    Some(seed)
}

fn seed_from_hash(hash: [u8; BLAKE2S_HASH_SIZE]) -> <StdRng as SeedableRng>::Seed {
    let mut seed = <StdRng as SeedableRng>::Seed::default();
    let seed_bytes = seed.as_mut();
    let len = seed_bytes.len().min(hash.len());

    seed_bytes[..len].copy_from_slice(&hash[..len]);
    seed
}
