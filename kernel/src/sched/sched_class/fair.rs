// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::binary_heap::BinaryHeap, sync::Arc};
use core::{
    cmp::{self, Reverse},
    sync::atomic::{AtomicU64, Ordering::*},
};

use ostd::{
    cpu::{num_cpus, CpuId},
    task::scheduler::{EnqueueFlags, UpdateFlags},
};

use super::{
    time::{base_slice_clocks, min_period_clocks},
    CurrentRuntime, SchedAttr, SchedClassRq,
};
use crate::{
    sched::priority::{Nice, NiceRange},
    thread::Thread,
};

const WEIGHT_0: u64 = 1024;
pub const fn nice_to_weight(nice: Nice) -> u64 {
    // Calculated by the formula below:
    //
    //     weight = 1024 * 1.25^(-nice)
    //
    // We propose that every increment of the nice value results
    // in 12.5% change of the CPU load weight.
    const FACTOR_NUMERATOR: u64 = 5;
    const FACTOR_DENOMINATOR: u64 = 4;

    const NICE_TO_WEIGHT: [u64; 40] = const {
        let mut ret = [0; 40];

        let mut index = 0;
        let mut nice = NiceRange::MIN;
        while nice <= NiceRange::MAX {
            ret[index] = match nice {
                0 => WEIGHT_0,
                nice @ 1.. => {
                    let numerator = FACTOR_DENOMINATOR.pow(nice as u32);
                    let denominator = FACTOR_NUMERATOR.pow(nice as u32);
                    WEIGHT_0 * numerator / denominator
                }
                nice => {
                    let numerator = FACTOR_NUMERATOR.pow((-nice) as u32);
                    let denominator = FACTOR_DENOMINATOR.pow((-nice) as u32);
                    WEIGHT_0 * numerator / denominator
                }
            };

            index += 1;
            nice += 1;
        }
        ret
    };

    NICE_TO_WEIGHT[(nice.range().get() + 20) as usize]
}

/// The scheduling entity for the FAIR scheduling class.
///
/// The structure contains a significant indicator: `vruntime`.
///
/// # `vruntime`
///
/// The vruntime (virtual runtime) is calculated by the formula:
///
///     vruntime += runtime_delta * WEIGHT_0 / weight
///
/// and a thread with a lower vruntime gains a greater privilege to be
/// scheduled, making the whole run queue balanced on vruntime (thus FAIR).
///
/// # Scheduling periods
///
/// Scheduling periods is designed to calculate the time slice for each threads.
///
/// The time slice for each threads is calculated by the formula:
///
///     time_slice = period * weight / total_weight
///
/// where `total_weight` is the sum of all weights in the run queue including
/// the current thread and [`period`](FairClassRq::period) is calculated
/// regarding the number of running threads.
///
/// When a thread meets the condition below, it will be preempted to the
/// run queue. See [`FairClassRq::update_current`] for more details.
///
///     period_delta > time_slice
///         || vruntime > rq_min_vruntime + normalized_time_slice
#[derive(Debug)]
pub struct FairAttr {
    weight: AtomicU64,
    vruntime: AtomicU64,
}

impl FairAttr {
    pub fn new(nice: Nice) -> Self {
        FairAttr {
            weight: nice_to_weight(nice).into(),
            vruntime: Default::default(),
        }
    }

    pub fn update(&self, nice: Nice) {
        self.weight.store(nice_to_weight(nice), Relaxed);
    }

    fn update_vruntime(&self, delta: u64) -> (u64, u64) {
        let weight = self.weight.load(Relaxed);
        let delta = delta * WEIGHT_0 / weight;
        let vruntime = self.vruntime.fetch_add(delta, Relaxed) + delta;
        (vruntime, weight)
    }
}

/// The wrapper for threads in the FAIR run queue.
///
/// This structure is used to provide the capability for keying in the
/// run queue implemented by `BTreeSet` in the `FairClassRq`.
struct FairQueueItem(Arc<Thread>);

impl core::fmt::Debug for FairQueueItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.key())
    }
}

impl FairQueueItem {
    fn key(&self) -> u64 {
        self.0.sched_attr().fair.vruntime.load(Relaxed)
    }
}

impl PartialEq for FairQueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.key().eq(&other.key())
    }
}

impl Eq for FairQueueItem {}

impl PartialOrd for FairQueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FairQueueItem {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

/// The per-cpu run queue for the FAIR scheduling class.
///
/// See [`FairAttr`] for the explanation of vruntimes and scheduling periods.
///
/// The structure contains a `BTreeSet` to store the threads in the run queue to
/// ensure the efficiency for finding next-to-run threads.
#[derive(Debug)]
pub(super) struct FairClassRq {
    #[allow(unused)]
    cpu: CpuId,
    /// The ready-to-run threads.
    threads: BinaryHeap<Reverse<FairQueueItem>>,
    /// The minimum of vruntime in the run queue. Serves as the initial
    /// value of newly-enqueued threads.
    min_vruntime: u64,
    total_weight: u64,
}

impl FairClassRq {
    pub fn new(cpu: CpuId) -> Self {
        Self {
            cpu,
            threads: BinaryHeap::new(),
            min_vruntime: 0,
            total_weight: 0,
        }
    }

    /// The scheduling period is calculated as the maximum of the following two values:
    ///
    /// 1. The minimum period value, defined by [`min_period_clocks`].
    /// 2. `period = min_granularity * n` where
    ///    `min_granularity = log2(1 + num_cpus) * base_slice_clocks`, and `n` is the number of
    ///    runnable threads (including the current running thread).
    ///
    /// The formula is chosen by 3 principles:
    ///
    /// 1. The scheduling period should reflect the running threads and CPUs;
    /// 2. The scheduling period should not be too low to limit the overhead of context switching;
    /// 3. The scheduling period should not be too high to ensure the scheduling latency
    ///    & responsiveness.
    fn period(&self) -> u64 {
        let base_slice_clks = base_slice_clocks();
        let min_period_clks = min_period_clocks();

        // `+ 1` means including the current running thread.
        let period_single_cpu =
            (base_slice_clks * (self.threads.len() + 1) as u64).max(min_period_clks);
        period_single_cpu * u64::from((1 + num_cpus()).ilog2())
    }

    /// The virtual time slice for each thread in the run queue, measured in vruntime clocks.
    fn vtime_slice(&self) -> u64 {
        self.period() / (self.threads.len() + 1) as u64
    }

    /// The time slice for each thread in the run queue, measured in sched clocks.
    fn time_slice(&self, cur_weight: u64) -> u64 {
        self.period() * cur_weight / (self.total_weight + cur_weight)
    }
}

impl SchedClassRq for FairClassRq {
    fn enqueue(&mut self, thread: Arc<Thread>, flags: Option<EnqueueFlags>) {
        let fair_attr = &thread.sched_attr().fair;
        let vruntime = match flags {
            Some(EnqueueFlags::Spawn) => self.min_vruntime + self.vtime_slice(),
            _ => self.min_vruntime,
        };
        fair_attr.vruntime.fetch_max(vruntime, Relaxed);

        self.total_weight += fair_attr.weight.load(Relaxed);
        self.threads.push(Reverse(FairQueueItem(thread)));
    }

    fn len(&mut self) -> usize {
        self.threads.len()
    }

    fn is_empty(&mut self) -> bool {
        self.threads.is_empty()
    }

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        let Reverse(FairQueueItem(thread)) = self.threads.pop()?;

        self.total_weight -= thread.sched_attr().fair.weight.load(Relaxed);

        Some(thread)
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        match flags {
            UpdateFlags::Yield => true,
            UpdateFlags::Tick | UpdateFlags::Wait => {
                let (vruntime, weight) = attr.fair.update_vruntime(rt.delta);
                self.min_vruntime = match self.threads.peek() {
                    Some(Reverse(leftmost)) => vruntime.min(leftmost.key()),
                    None => vruntime,
                };

                rt.period_delta > self.time_slice(weight)
                    || vruntime > self.min_vruntime + self.vtime_slice()
            }
        }
    }
}
