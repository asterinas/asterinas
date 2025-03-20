// SPDX-License-Identifier: MPL-2.0

use core::{
    ops::Add,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

use super::{fair::FairClassRq, SchedClassRq, SchedPolicyKind};
use crate::thread::Thread;

// Using a 2-based factor to quicken the calculation process.
const PERIOD_NS: u64 = 1 << 30;

const POW_FACTOR: usize = 32;
const SHL_FACTOR: u32 = u32::BITS;

// The actual storage is `|i| y^i * 2^SHL_FACTOR` where `y = 0.5^(1 / POW_FACTOR)`.
// Use this method to avoid floating point operations while preserving
// decent precision.
const Y_POW_SHL: [u64; POW_FACTOR] = {
    let y_p32shl32 = 1u64 << (SHL_FACTOR - 1);
    let y_p16shl32 = (y_p32shl32 << SHL_FACTOR).isqrt();
    let y_p8shl32 = (y_p16shl32 << SHL_FACTOR).isqrt();
    let y_p4shl32 = (y_p8shl32 << SHL_FACTOR).isqrt();
    let y_p2shl32 = (y_p4shl32 << SHL_FACTOR).isqrt();
    let y_p1shl32 = (y_p2shl32 << SHL_FACTOR).isqrt();

    let values = [y_p1shl32, y_p2shl32, y_p4shl32, y_p8shl32, y_p16shl32];

    let mut table = [1 << SHL_FACTOR; POW_FACTOR];
    let mut i = 1;
    while i < POW_FACTOR {
        let mut bits = i;
        let mut j = 0;
        while bits != 0 {
            if bits & 1 != 0 {
                table[i] = (table[i] * values[j]) >> SHL_FACTOR;
            }
            bits >>= 1;
            j += 1;
        }
        i += 1;
    }
    table
};

const Y_SHL: u64 = Y_POW_SHL[1];

/// Calculates `x * y^n`.
fn mul_y_pow(x: u64, n: u64) -> u64 {
    let period = n / POW_FACTOR as u64;
    let index = n % POW_FACTOR as u64;

    if period > u64::from(u64::BITS) {
        return 0;
    }

    ((u128::from(x >> period) * u128::from(Y_POW_SHL[index as usize])) >> SHL_FACTOR) as u64
}

/// `T / (1 - y)`.
const PERIOD_DIV_1_MINUS_Y_NS: u64 = (PERIOD_NS << SHL_FACTOR) / ((1 << SHL_FACTOR) - Y_SHL);

/// Calculates `F(p, d1, d3)`
fn load_terms(p: u64, d1: u64, d3: u64) -> u64 {
    let c1 = mul_y_pow(d1, p);
    let c2 = PERIOD_DIV_1_MINUS_Y_NS - mul_y_pow(PERIOD_DIV_1_MINUS_Y_NS, p) - PERIOD_NS;
    let c3 = d3;
    c1 + c2 + c3
}

/// Calculates `R(t + dt)`.
fn avg_divider(d3: u64) -> u64 {
    PERIOD_DIV_1_MINUS_Y_NS - PERIOD_NS + d3
}

#[derive(Debug, Clone, Copy)]
pub struct LoadData {
    pub total_weight: u64,
    pub queued: u64,
    pub running: u64,
}

impl Add for LoadData {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            total_weight: self.total_weight + rhs.total_weight,
            queued: self.queued + rhs.queued,
            running: self.running + rhs.running,
        }
    }
}

#[derive(Debug)]
struct Load {
    last_updated: AtomicU64,
    d3: AtomicU64,

    weighted_sum: AtomicU64,
    queued_sum: AtomicU64,
    running_sum: AtomicU64,

    weighted_avg: AtomicU64,
    queued_avg: AtomicU64,
    running_avg: AtomicU64,
}

impl Load {
    pub const fn new() -> Self {
        Self {
            last_updated: AtomicU64::new(0),
            d3: AtomicU64::new(0),
            weighted_sum: AtomicU64::new(0),
            queued_sum: AtomicU64::new(0),
            running_sum: AtomicU64::new(0),
            weighted_avg: AtomicU64::new(0),
            queued_avg: AtomicU64::new(0),
            running_avg: AtomicU64::new(0),
        }
    }

    pub fn data(&self) -> LoadData {
        LoadData {
            total_weight: self.weighted_avg.load(Relaxed),
            queued: self.queued_avg.load(Relaxed),
            running: self.running_avg.load(Relaxed),
        }
    }

    /// Updates the load sums.
    ///
    /// # Returns
    ///
    /// Returns whether the period is decayed and the load averages are updated.
    pub fn update(
        &self,
        now_ns: u64,
        load: u64,
        weight: u64,
        queued: usize,
        running: bool,
    ) -> bool {
        // This is used to preserve the precision for queued & running sums.
        const CAPACITY_SHIFT: u32 = 10;

        let last_updated = self.last_updated.load(Relaxed);
        self.last_updated.store(now_ns, Relaxed);

        if now_ns < last_updated {
            return false;
        }

        let dt = now_ns - last_updated;
        let d3 = now_ns % PERIOD_NS;
        let d1 = (dt + PERIOD_NS - d3) % PERIOD_NS;
        let p = (dt + PERIOD_NS - d3) / PERIOD_NS;

        let mut delta = d3;
        self.d3.store(d3, Relaxed);

        // Decay the load sums.
        let decayed = p > 0;
        if decayed {
            self.weighted_sum
                .store(mul_y_pow(self.weighted_sum.load(Relaxed), p), Relaxed);
            self.queued_sum
                .store(mul_y_pow(self.queued_sum.load(Relaxed), p), Relaxed);
            self.running_sum
                .store(mul_y_pow(self.running_sum.load(Relaxed), p), Relaxed);

            if load > 0 || queued > 0 || running {
                delta = load_terms(p, d1, d3);
            }
        }

        // Update the load sums.
        if load > 0 {
            self.weighted_sum.fetch_add(delta * load, Relaxed);
        }
        if queued > 0 {
            self.queued_sum
                .fetch_add(delta * (queued as u64) << CAPACITY_SHIFT, Relaxed);
        }
        if running {
            self.running_sum.fetch_add(delta << CAPACITY_SHIFT, Relaxed);
        }

        // Update the load averages.
        if decayed {
            let r = avg_divider(self.d3.load(Relaxed));

            self.weighted_avg
                .store(weight * self.weighted_sum.load(Relaxed) / r, Relaxed);
            self.queued_avg
                .store(self.queued_sum.load(Relaxed) / r, Relaxed);
            self.running_avg
                .store(self.running_sum.load(Relaxed) / r, Relaxed);
        }

        decayed
    }
}

// Task-level load tracking

#[derive(Debug)]
pub struct FairEntityLoad(Load);

impl FairEntityLoad {
    pub const fn new() -> Self {
        FairEntityLoad(Load::new())
    }

    #[expect(unused)]
    pub fn data(&self) -> LoadData {
        self.0.data()
    }

    pub fn update(
        &self,
        now_ns: u64,
        has_been_runnable: bool,
        has_been_running: bool,
        weight: u64,
    ) -> bool {
        self.0.update(
            now_ns,
            has_been_runnable as u64,
            weight,
            has_been_runnable as usize,
            has_been_running,
        )
    }
}

// Class-level load tracking

#[derive(Debug)]
pub struct FairRqLoad(Load);

impl FairRqLoad {
    pub const fn new() -> Self {
        FairRqLoad(Load::new())
    }

    #[expect(unused)]
    pub fn data(&self) -> LoadData {
        self.0.data()
    }

    pub fn update(&mut self, now_ns: u64, cur: Option<&Thread>, rq: &FairClassRq) -> bool {
        let is_running =
            cur.is_some_and(|cur| cur.sched_attr().policy_kind() == SchedPolicyKind::Fair);

        self.0.update(
            now_ns,
            rq.total_weight(),
            1,
            rq.len() + is_running as usize,
            is_running,
        )
    }

    /// # Invariant
    ///
    /// The update time of this load measurement must be later than that of the entity.
    pub fn attach(&mut self, FairEntityLoad(ent): &FairEntityLoad, entity_weight: u64) {
        let this = &mut self.0;

        debug_assert!(this.last_updated.load(Relaxed) >= ent.last_updated.load(Relaxed));

        let d3 = this.d3.load(Relaxed);
        let divider = avg_divider(d3);

        // Sync the update time.

        ent.last_updated
            .store(this.last_updated.load(Relaxed), Relaxed);
        ent.d3.store(d3, Relaxed);

        // Recalculate the load sums. Here comes some precision loss since the period
        // were not synced, but the averages should be approximate enough.

        let weighted_sum = ent.weighted_avg.load(Relaxed) * divider;
        ent.weighted_sum
            .store((weighted_sum / entity_weight).max(1), Relaxed);

        ent.queued_sum
            .store(ent.queued_avg.load(Relaxed) * divider, Relaxed);
        ent.running_sum
            .store(ent.running_avg.load(Relaxed) * divider, Relaxed);

        // Append the data to the class-level load measurement.

        this.weighted_sum
            .fetch_add(ent.weighted_sum.load(Relaxed) * entity_weight, Relaxed);
        this.weighted_avg
            .fetch_add(ent.weighted_avg.load(Relaxed), Relaxed);

        this.queued_sum
            .fetch_add(ent.queued_sum.load(Relaxed), Relaxed);
        this.queued_avg
            .fetch_add(ent.queued_avg.load(Relaxed), Relaxed);

        this.running_sum
            .fetch_add(ent.running_sum.load(Relaxed), Relaxed);
        this.running_avg
            .fetch_add(ent.running_avg.load(Relaxed), Relaxed);
    }

    /// # Invariant
    ///
    /// The update time of this load measurement must be synced with that of the entity.
    pub fn detach(&mut self, FairEntityLoad(ent): &FairEntityLoad, entity_weight: u64) {
        let this = &mut self.0;

        debug_assert!(this.last_updated.load(Relaxed) >= ent.last_updated.load(Relaxed));

        // Remove the data from the class-level load measurement.

        this.weighted_sum
            .fetch_sub(ent.weighted_sum.load(Relaxed) * entity_weight, Relaxed);
        this.weighted_avg
            .fetch_sub(ent.weighted_avg.load(Relaxed), Relaxed);

        this.queued_sum
            .fetch_sub(ent.queued_sum.load(Relaxed), Relaxed);
        this.queued_avg
            .fetch_sub(ent.queued_avg.load(Relaxed), Relaxed);

        this.running_sum
            .fetch_sub(ent.running_sum.load(Relaxed), Relaxed);
        this.running_avg
            .fetch_sub(ent.running_avg.load(Relaxed), Relaxed);
    }
}

#[derive(Debug)]
pub struct RqLoad {
    inner: Load,
    kind: SchedPolicyKind,
}

impl RqLoad {
    pub const fn new(kind: SchedPolicyKind) -> Self {
        RqLoad {
            inner: Load::new(),
            kind,
        }
    }

    #[expect(unused)]
    pub fn data(&self) -> LoadData {
        self.inner.data()
    }

    pub fn update(&mut self, now_ns: u64, cur: Option<&Thread>) -> bool {
        let is_running = cur.is_some_and(|cur| cur.sched_attr().policy_kind() == self.kind);

        self.inner.update(
            now_ns,
            is_running as u64,
            1,
            is_running as usize,
            is_running,
        )
    }
}
