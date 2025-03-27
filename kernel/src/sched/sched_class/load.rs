// SPDX-License-Identifier: MPL-2.0

use core::{
    cell::Cell,
    ops::Add,
    sync::{
        atomic::{AtomicU64, Ordering::Relaxed},
        Exclusive,
    },
};

use radium::Radium;

use super::{fair::FairClassRq, SchedClassRq, SchedPolicyKind};
use crate::thread::Thread;

// Using a 2-based factor to quicken the calculation process.
/// The period is 1048576 ns (approximately 1 ms).
const PERIOD_NS: u64 = 1 << 20;

const POW_FACTOR: usize = 32;
const SHL_FACTOR: u32 = u32::BITS;

// The actual storage is `|i| y^i * 2^SHL_FACTOR` where `y = 0.5^(1 / POW_FACTOR)`.
// Use this method to avoid floating point operations while preserving
// decent precision.
const Y_POW_SHL: [u64; POW_FACTOR] = {
    let y_p32shl32 = 1u64 << (SHL_FACTOR - 1);
    // y^16 << SHL = sqrt((y^32 << SHL) << SHL). The rest are the same.
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
                // Accumulates the contribution of every bit in the power factor i.
                // y^(a + b) << SHL = ((y^a << SHL) * (y^b << SHL)) >> SHL.
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

    if period >= u64::from(u64::BITS) {
        return 0;
    }

    // x * y^n = ((x >> (n / POW)) * Y_POW_SHL[n % POW]) >> SHL.
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
struct Load<T: Radium<Item = u64>> {
    last_updated: T,
    d3: T,

    weighted_sum: T,
    queued_sum: T,
    running_sum: T,

    weighted_avg: T,
    queued_avg: T,
    running_avg: T,
}

impl<T: Radium<Item = u64>> Load<T> {
    pub fn new() -> Self {
        Self {
            last_updated: T::new(0),
            d3: T::new(0),
            weighted_sum: T::new(0),
            queued_sum: T::new(0),
            running_sum: T::new(0),
            weighted_avg: T::new(0),
            queued_avg: T::new(0),
            running_avg: T::new(0),
        }
    }

    pub fn data(&self) -> LoadData {
        LoadData {
            total_weight: self.weighted_avg.load(Relaxed),
            queued: self.queued_avg.load(Relaxed),
            running: self.running_avg.load(Relaxed),
        }
    }

    /// Updates the load measurement.
    ///
    /// # Arguments
    ///
    /// The `load`, `queued` and `running` arguments are markers at the end of
    /// the latest measurement period instead of the reflection of the current
    /// state.
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

/// The load measurement for a FAIR task.
#[derive(Debug)]
pub struct FairTaskLoad(Load<AtomicU64>);

impl FairTaskLoad {
    pub fn new() -> Self {
        FairTaskLoad(Load::new())
    }

    #[expect(unused)]
    pub fn data(&self) -> LoadData {
        self.0.data()
    }

    /// Updates the load measurement for a FAIR task.
    ///
    /// # Arguments
    ///
    /// The `runnable` and `running` arguments are markers at the end of the latest
    /// measurement period instead of the reflection of the current state.
    pub fn update(&self, now_ns: u64, runnable: bool, running: bool, weight: u64) -> bool {
        self.0
            .update(now_ns, runnable as u64, weight, runnable as usize, running)
    }
}

// Class-level load tracking

// The use of `Exclusive` here is to implement `Sync` for `FairRqLoad`. `Cell<T>`
// is not `Sync` by definition because of its mutability under shared references.
// `Exclusive<T>` wraps `Cell<T>` and provides only mutable reference to its inner
// value, which prevents shared references and becomes `Sync`.

/// The load measurement for a FAIR run queue.
#[derive(Debug)]
pub struct FairRqLoad(Exclusive<Load<Cell<u64>>>);

impl FairRqLoad {
    pub fn new() -> Self {
        FairRqLoad(Exclusive::new(Load::new()))
    }

    #[expect(unused)]
    pub fn data(&mut self) -> LoadData {
        self.0.get_mut().data()
    }

    /// Updates the load measurement for a FAIR run queue.
    ///
    /// This function must be called before any subsequent changes to the current state.
    pub fn update(&mut self, now_ns: u64, cur: Option<&Thread>, rq: &FairClassRq) -> bool {
        let running =
            cur.is_some_and(|cur| cur.sched_attr().policy_kind() == SchedPolicyKind::Fair);

        self.0.get_mut().update(
            now_ns,
            rq.total_weight(),
            1,
            rq.len() + running as usize,
            running,
        )
    }

    /// Attaches a task's load measurement to this run queue's load measurement.
    ///
    /// The attachment & detachment are needed even if the `total_weight` and the number
    /// of the tasks are already recorded in the run queue's measurement. This is because
    /// the load averages take the historical (though decayed) data into account, and
    /// simply recording those data cannot reflect the change. By syncing the measurement
    /// of tasks with that of the run queue, the load averages will be recalculated to
    /// reflect the change.
    ///
    /// # Invariant
    ///
    /// The update time of this load measurement must be later than that of the entity.
    pub fn attach(&mut self, FairTaskLoad(ent): &FairTaskLoad, entity_weight: u64) {
        let this = &mut self.0.get_mut();

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

    /// Detaches a task's load measurement from this run queue's load measurement.
    ///
    /// # Invariant
    ///
    /// The update time of this load measurement must be synced with that of the entity.
    pub fn detach(&mut self, FairTaskLoad(ent): &FairTaskLoad, entity_weight: u64) {
        let this = &mut self.0.get_mut();

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

/// The load measurement of a non-FAIR run queue.
#[derive(Debug)]
pub struct RqLoad {
    inner: Exclusive<Load<Cell<u64>>>,
    kind: SchedPolicyKind,
}

impl RqLoad {
    pub fn new(kind: SchedPolicyKind) -> Self {
        debug_assert!(
            !matches!(kind, SchedPolicyKind::Fair),
            "RqLoad is not for FAIR run queues",
        );
        RqLoad {
            inner: Exclusive::new(Load::new()),
            kind,
        }
    }

    #[expect(unused)]
    pub fn data(&mut self) -> LoadData {
        self.inner.get_mut().data()
    }

    /// Updates the load measurement for a non-FAIR run queue.
    ///
    /// This function must be called before any subsequent changes to the current state.
    pub fn update(&mut self, now_ns: u64, cur: Option<&Thread>) -> bool {
        let running = cur.is_some_and(|cur| cur.sched_attr().policy_kind() == self.kind);

        (self.inner.get_mut()).update(now_ns, running as u64, 1, running as usize, running)
    }
}
