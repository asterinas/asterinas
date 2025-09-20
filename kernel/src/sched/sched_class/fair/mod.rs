// SPDX-License-Identifier: MPL-2.0

//! # An Earliest Eligible Virtual Deadline First (EEVDF) scheduler
//!
//! EEVDF is a task scheduling algorithm that can model the CPU demand of latency-sensitive
//! tasks, mixing deadline greediness and an eligibility criterion in order to achieve
//! fairness.
//!
//! ## Basic concepts
//!
//! This section offers a brief introduction to a few key concepts involved in the EEVDF
//! algorithm.
//!
//! ### Nice values and weights
//!
//! Similar to Linux, Asterinas associates each task with a *nice value*, which represents
//! its relative priority. Nice values are mapped to scheduling *weights*, where a lower nice
//! value corresponds to a higher weight. Scheduling weights determine the proportion of CPU
//! time allocated to a task relative to others. For example, a task with twice the weight of
//! another is expected to receive roughly twice as much CPU time over the long run.
//!
//! ### Virtual time
//!
//! To reason about fairness and latency independently of wall-clock time, EEVDF uses the
//! notion of *virtual time*. Virtual time advances at a rate inversely proportional to a task’s
//! weight: tasks with higher weights accumulate virtual runtime more slowly, reflecting the
//! fact that they are entitled to a larger share of CPU. This abstraction allows the scheduler
//! to compare time-related quantities (such as deadlines) across tasks of different priorities.
//!
//! ### Virtual deadline
//!
//! EEVDF assigns a base time slice for each task, which is converted to a virtual time slice.
//! Thus, tasks with higher weights end up with shorter virtual time slices.
//!
//! The point in virtual time where the slice ends is the *virtual deadline*.
//!
//! ### Lag
//!
//! *Lag* encapsulates the idea of how much CPU time the scheduler owes to a task. To quantify
//! the notion of lag, EEVDF subtracts the system's virtual runtime by the task's. Since virtual
//! time for tasks with higher weights advances slower, such tasks are more likely to acquire
//! higher lag.
//!
//! In the "Design" section we'll see how lag can influence deadlines and eligibility more precisely.
//!
//! ### Eligibility
//!
//! Having the earliest virtual deadline is not the only criterion for a task to be picked next.
//! EEVDF picks the task with the earliest virtual deadline among the tasks that are *eligible*.
//!
//! A task is eligible if its lag is non-negative. That is, it's either even with the system's
//! virtual runtime or it's owed CPU.
//!
//! ## Design
//!
//! Linux's EEVDF reference implementation approximates the system's virtual runtime by the weighted
//! average of the virtual runtime of all tasks. To prevent overflows and loss of precision, however,
//! this quantity is not stored directly. Instead, for the set S of tasks, the scheduler maintains
//!
//! * The total weight of all tasks, W = ∑{i ∈ S}{wᵢ}
//! * The minimum virtual runtime across all tasks, ρₘᵢₙ
//! * The weighted sum of virtual runtime offsets, Φ = ∑{i ∈ S}{wᵢ(ρᵢ - ρₘᵢₙ)}
//!
//! When necessary, the weighted average virtual runtime can be computed as ρₐᵥᵣ = Φ / W + ρₘᵢₙ.
//!
//! Part of the complexity comes from keeping those updated. Especially Φ, which will be explained
//! in detail.
//!
//! ### Enqueuing a task – `SchedClassRq::enqueue`
//!
//! The first thing needed for an enqueued task t is a stable choice of its virtual runtime ρₜ.
//!
//! EEVDF places it at the average virtual runtime subtracted by the task's virtual lag. There's a
//! caveat about the virtual lag needing an adjustment, but this detail can be discussed later.
//!
//! Once ρₜ is chosen, the virtual deadline for t is defined as ρₜ + qW₀ / wₜ, where q is the base
//! time slice, W₀ is a constant to mitigate loss of precision and wₜ is the weight of the task t.
//!
//! It's worth noting that:
//! * The greater the lag, the smaller ρₜ, the earlier the virtual deadline
//! * The greater the weight, the smaller qW₀ / wₜ, the earlier the virtual deadline
//!
//! Now that the deadline is defined, the task can be inserted in the eligibility queue (more on
//! this later).
//!
//! Finally, compute the updated Φ' as
//!
//! ```text
//! Φ' = ∑{i ∈ SU⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ')}
//!    = ∑{i ∈ S}{wᵢ(ρᵢ - ρₘᵢₙ')} + wₜ(ρₜ - ρₘᵢₙ')
//!    = ∑{i ∈ S}{wᵢ(ρᵢ - ρₘᵢₙ)} - ∑{i ∈ S}{wᵢ(ρₘᵢₙ' - ρₘᵢₙ)} + wₜ(ρₜ - ρₘᵢₙ')
//!    = Φ + W(ρₘᵢₙ - ρₘᵢₙ') + wₜ(ρₜ - ρₘᵢₙ')
//! ```
//!
//! Notably, there are two special cases:
//! * If the new minimum virtual runtime doesn't change, ρₘᵢₙ - ρₘᵢₙ' = 0
//! * If t has the new minimum virtual runtime, ρₜ - ρₘᵢₙ' = 0
//!
//! ### Choosing the next task – `SchedClassRq::pick_next`
//!
//! Start by popping the next task from the eligibility queue. Before setting it as the current
//! task, some bookkeeping is required.
//!
//! The task t being rescheduled needs to have its virtual lag stored, computed as ρₐᵥᵣ - ρₜ.
//! Here, Linux's EEVDF [clamps the virtual lag](https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L686)
//! for stability. Though, for a small optimization, none of this is necessary when the task is
//! exiting.
//!
//! Then, compute the updated Φ' as
//!
//! ```text
//! Φ' = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ')}
//!    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ)} - ∑{i ∈ S\⦃t⦄}{wᵢ(ρₘᵢₙ' - ρₘᵢₙ)}
//!    = Φ - wₜ(ρₜ - ρₘᵢₙ) - W'(ρₘᵢₙ' - ρₘᵢₙ)
//! ```
//!
//! Again, there are two special cases:
//! * If the new minimum virtual runtime doesn't change, ρₘᵢₙ - ρₘᵢₙ' = 0
//! * If t had the minimum virtual runtime, ρₜ - ρₘᵢₙ = 0
//!
//! ### Updating the current task – `SchedClassRq::update_current`
//!
//! When the current task t is being updated after some wall-clock time δ, translate it to
//! virtual time: Δ = δW₀ / wₜ.
//!
//! The new virtual runtime for t is updated as ρₜ' = ρₜ + Δ.
//!
//! Then, compute the updated Φ' as
//!
//! ```text
//! Φ' = ∑{i ∈ S}{wᵢ(ρᵢ' - ρₘᵢₙ')}
//!    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ')} + wₜ(ρₜ + Δ - ρₘᵢₙ')
//!    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ + ρₘᵢₙ - ρₘᵢₙ')} + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
//!    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ)} + (ρₘᵢₙ - ρₘᵢₙ')∑{i ∈ S\⦃t⦄}{wᵢ} + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
//!    = Φ - wₜ(ρₜ - ρₘᵢₙ) + (ρₘᵢₙ - ρₘᵢₙ')(W - wₜ) + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
//!    = Φ + wₜΔ - W(ρₘᵢₙ' - ρₘᵢₙ)
//! ```
//!
//! If the minimum virtual runtime doesn't change, then ρₘᵢₙ' - ρₘᵢₙ = 0.
//!
//! And finally, to know whether the current task needs to be rescheduled or not, the rules are:
//! 1. If the eligibility queue is empty, then no. Otherwise,
//! 2. If the current task is going to sleep or exiting, then yes. Otherwise,
//! 3. If the current task is yielding or just ticking, return whether its allocated service has
//!    been fulfilled or not. That is, whether its virtual runtime has reached its virtual deadline.
//!
//! ### Adjusting the virtual lag when enqueuing a task
//!
//! The [Linux source](https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L5230)
//! explains this in detail. But for the scope of this documentation, it's enough to reuse some
//! of Linux's wording and state that to prevent virtual lag of a task t of weight wₜ from quickly
//! evaporating, it needs to be multiplied by (W + wₜ) / W.
//!
//! ### The eligibility queue
//!
//! The eligibility queue is, for the most part, a normal balanced tree whose nodes are keyed by
//! the tasks' virtual deadlines and some unique ID for tie-breaking.
//!
//! However, while it's being traversed from top to bottom in search of the next task to pop,
//! branches without eligible tasks can be pruned altogether with a smart trick.
//!
//! From the virtual lag formula, a task t is eligible when ρₐᵥᵣ - ρₜ ≥ 0. Substituting ρₐᵥᵣ:
//!
//! ```text
//! Φ / W + ρₘᵢₙ - ρₜ ≥ 0 => (ρ - ρₘᵢₙ)W ≤ Φ
//! ```
//!
//! Then, each node is augmented with the minimum virtual runtime across its own task and the
//! tasks of its children. If the minimum virtual runtime of a branch is not small enough for the
//! eligibility check to return `true`, then such branch doesn't have any eligible task.
//!
//! So the traversal order is:
//! 1. If the left child has an eligible task, find the task with the earliest deadline there. Otherwise,
//! 2. If the task of the current node is eligible, choose it. Otherwise,
//! 3. If the right child has an eligible task, find the task with the earliest deadline there. Otherwise,
//! 4. Just return the leftmost task, ignoring eligibility.
//!
//! The 4th rule is needed when the queue doesn't have any eligible task but the current task is
//! going to sleep or exiting, forcing the algorithm to pick an ineligible task in order to avoid contention.
//!
//! ## Prior art and references
//!
//! * https://people.eecs.berkeley.edu/~istoica/papers/eevdf-tr-95.pdf
//! * https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c

mod queue;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicI64, Ordering};

use ostd::{
    cpu::CpuId,
    task::{
        scheduler::{EnqueueFlags, UpdateFlags},
        Task,
    },
};

use crate::{
    sched::{
        nice::{Nice, NiceValue},
        sched_class::{
            fair::queue::{EligibilityQueue, TaskData},
            time::{base_slice_clocks, tick_period_clocks},
            CurrentRuntime, SchedAttr, SchedClassRq,
        },
    },
    thread::AsThread,
};

const WEIGHT_0: i64 = 1024;

const fn nice_to_weight(nice: Nice) -> i64 {
    // Calculated by the formula below:
    //
    //     weight = 1024 * 1.25^(-nice)
    //
    // We propose that every increment of the nice value results
    // in 12.5% change of the CPU load weight.
    const FACTOR_NUMERATOR: i64 = 5;
    const FACTOR_DENOMINATOR: i64 = 4;

    const NICE_TO_WEIGHT: [i64; 40] = const {
        let mut ret = [0; 40];

        let mut index = 0;
        let mut nice = NiceValue::MIN.get();
        while nice <= NiceValue::MAX.get() {
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

    NICE_TO_WEIGHT[(nice.value().get() + 20) as usize]
}

/// Converts wall-clock time to virtual time.
fn wall_to_virtual(delta: i64, weight: i64) -> i64 {
    if weight != WEIGHT_0 {
        // TODO: set as cold path.
        delta * WEIGHT_0 / weight // `weight` can never be zero.
    } else {
        delta // Avoid unnecessary math most of the times.
    }
}

/// Computes the weighted average vruntime: Φ / W + ρₘᵢₙ
///
/// This function panics if `total_weight` is zero.
fn avg_vruntime_unchecked(
    mut weighted_vruntime_offsets: i64,
    total_weight: i64,
    min_vruntime: i64,
) -> i64 {
    if weighted_vruntime_offsets < 0 {
        // Sign flips effective floor/ceiling.
        weighted_vruntime_offsets -= total_weight - 1;
    }
    weighted_vruntime_offsets / total_weight + min_vruntime
}

/// Computes the weighted average vruntime: Φ / W + ρₘᵢₙ.
/// If W is zero, no task is included. In this case, just return ρₘᵢₙ.
fn avg_vruntime(weighted_vruntime_offsets: i64, total_weight: i64, min_vruntime: i64) -> i64 {
    if total_weight != 0 {
        avg_vruntime_unchecked(weighted_vruntime_offsets, total_weight, min_vruntime)
    } else {
        min_vruntime
    }
}

/// The scheduling attribute for the FAIR scheduling class.
///
/// Some attributes are derived/set from outside callers and others are meant for
/// post-wakeup persistency.
#[derive(Debug)]
pub struct FairAttr {
    /// The task weight is derived from it's associated [`Nice`] value.
    /// By construction, the weight is always positive.
    weight: AtomicI64,
    /// The virtual lag attributed to a task when it's rescheduled.
    /// If the task is exiting, the virtual lag is not updated.
    vlag: AtomicI64,
}

impl FairAttr {
    pub fn new(nice: Nice) -> Self {
        FairAttr {
            weight: nice_to_weight(nice).into(),
            vlag: AtomicI64::new(0),
        }
    }

    pub fn update(&self, nice: Nice) {
        self.weight.store(nice_to_weight(nice), Ordering::Release);
    }
}

/// A fair run queue that implements the EEVDF scheduling algorithm. Refer to the
/// module docstring for detailed information.
#[derive(Debug)]
pub(super) struct FairClassRq {
    #[expect(unused)]
    cpu: CpuId,
    queue: EligibilityQueue<Arc<Task>>,
    queue_len: usize,
    queued_weight: i64,
    weighted_vruntime_offsets: i64,
    current_task_data: Option<TaskData<Arc<Task>>>,
    next_id: u64,
    base_slice_clocks: i64,
    lag_limit_clocks: i64,
}

impl FairClassRq {
    pub fn new(cpu: CpuId) -> Self {
        let base_slice_clocks = base_slice_clocks() as i64;

        // Reference: https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L686
        let lag_limit_clocks = (tick_period_clocks() as i64).max(2 * base_slice_clocks);

        Self {
            cpu,
            queue: EligibilityQueue::new(),
            queue_len: 0,
            queued_weight: 0,
            weighted_vruntime_offsets: 0,
            current_task_data: None,
            next_id: 0,
            base_slice_clocks,
            lag_limit_clocks,
        }
    }

    /// Returns the minimum vruntime across all scheduled tasks, including the
    /// current task.
    fn min_vruntime(&self) -> i64 {
        match (&self.current_task_data, self.queue.min_vruntime()) {
            (None, None) => 0,
            (None, Some(x)) => x,
            (Some(current_task_data), None) => current_task_data.vruntime,
            (Some(current_task_data), Some(y)) => current_task_data.vruntime.min(y),
        }
    }

    /// Returns the total scheduled weight, including the current task.
    fn total_weight(&self) -> i64 {
        match &self.current_task_data {
            Some(current_task_data) => current_task_data.weight + self.queued_weight,
            None => self.queued_weight,
        }
    }
}

impl SchedClassRq for FairClassRq {
    fn enqueue(&mut self, task: Arc<Task>, flags: Option<EnqueueFlags>) {
        // Φ' = ∑{i ∈ S∪⦃t⦄}wᵢ(ρᵢ - ρₘᵢₙ')}
        //    = ∑{i ∈ S}{wᵢ(ρᵢ - ρₘᵢₙ')} + wₜ(ρₜ - ρₘᵢₙ')
        //    = ∑{i ∈ S}{wᵢ(ρᵢ - ρₘᵢₙ)} - ∑{i ∈ S}{wᵢ(ρₘᵢₙ' - ρₘᵢₙ)} + wₜ(ρₜ - ρₘᵢₙ')
        //    = Φ + W(ρₘᵢₙ - ρₘᵢₙ') + wₜ(ρₜ - ρₘᵢₙ')

        let fair_attr = &task.as_thread().unwrap().sched_attr().fair;

        let weight = fair_attr.weight.load(Ordering::Relaxed);
        let mut vslice = wall_to_virtual(self.base_slice_clocks, weight);

        let min_vruntime = self.min_vruntime();
        let total_weight = self.total_weight();
        let vruntime = match flags {
            Some(EnqueueFlags::Spawn) => {
                // When joining the competition; the existing tasks will be,
                // on average, halfway through their slice, as such start tasks
                // off with half a slice to ease into the competition.
                // Reference: https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L5300
                vslice >>= 1;

                avg_vruntime(self.weighted_vruntime_offsets, total_weight, min_vruntime)
            }
            _ => {
                if total_weight != 0 {
                    // Load the stored virtual lag.
                    let vlag = fair_attr.vlag.load(Ordering::Relaxed);
                    // If we want to place a task and preserve lag, we have to
                    // consider the effect of the new entity on the weighted
                    // average and compensate for this, otherwise lag can quickly
                    // evaporate.
                    // Reference: https://elixir.bootlin.com/linux/v6.16/source/kernel/sched/fair.c#L5230
                    let vlag_adjusted = (total_weight + weight) * vlag / total_weight;

                    let avg_vruntime = avg_vruntime_unchecked(
                        self.weighted_vruntime_offsets,
                        total_weight,
                        min_vruntime,
                    );

                    avg_vruntime - vlag_adjusted
                } else {
                    // The task is somehow alone. Lag is irrelevant.
                    min_vruntime
                }
            }
        };

        if vruntime < min_vruntime {
            // ρₜ = ρₘᵢₙ' => Φ' = Φ + W(ρₘᵢₙ - ρₘᵢₙ')
            self.weighted_vruntime_offsets += total_weight * (min_vruntime - vruntime);
        } else {
            // ρₘᵢₙ' = ρₘᵢₙ => Φ' = Φ + wₜ(ρₜ - ρₘᵢₙ')
            self.weighted_vruntime_offsets += weight * (vruntime - min_vruntime);
        }

        // Define the virtual deadline.
        let vdeadline = vruntime + vslice;

        // Define the ID for newly enqueued tasks.
        let id = self.next_id;
        self.next_id += 1;

        self.queue.push(TaskData {
            task,
            vdeadline,
            id,
            weight,
            vruntime,
            is_exiting: false,
        });

        self.queue_len += 1;
        self.queued_weight += weight;
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        // Φ' = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ')}
        //    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ)} - ∑{i ∈ S\⦃t⦄}{wᵢ(ρₘᵢₙ' - ρₘᵢₙ)}
        //    = Φ - wₜ(ρₜ - ρₘᵢₙ) - W'(ρₘᵢₙ' - ρₘᵢₙ)

        let min_vruntime = self.min_vruntime();
        let total_weight = self.total_weight();
        let TaskData {
            task,
            id,
            vdeadline,
            weight,
            vruntime,
            is_exiting,
        } = self
            .queue
            .pop(min_vruntime, total_weight, self.weighted_vruntime_offsets)?;

        if let Some(TaskData {
            task: resched_task,
            vruntime: resched_vruntime,
            weight: resched_weight,
            is_exiting: resched_is_exiting,
            ..
        }) = self.current_task_data.take()
        {
            if !resched_is_exiting {
                // Store the virtual lag for the rescheduled task.
                let avg_vruntime =
                    avg_vruntime(self.weighted_vruntime_offsets, total_weight, min_vruntime);
                let vlimit = wall_to_virtual(self.lag_limit_clocks, weight);
                let vlag = (avg_vruntime - resched_vruntime).clamp(-vlimit, vlimit);
                let resched_fair_attr = &resched_task.as_thread().unwrap().sched_attr().fair;
                resched_fair_attr.vlag.store(vlag, Ordering::Relaxed);
            }

            // This is the minimum queued vruntime *before* popping.
            let min_queued_vruntime = self.queue.min_vruntime_against(vruntime);

            if resched_vruntime < min_queued_vruntime {
                // `resched_vruntime` was the minimum vruntime and now it's moved
                // forward to `min_queued_vruntime`.
                // ρₜ = ρₘᵢₙ => Φ' = Φ - W'(ρₘᵢₙ' - ρₘᵢₙ)
                self.weighted_vruntime_offsets -=
                    self.queued_weight * (min_queued_vruntime - resched_vruntime);
            } else {
                // `min_queued_vruntime` remains as the minimum vruntime.
                // ρₘᵢₙ' = ρₘᵢₙ =>  Φ' = Φ - wₜ(ρₜ - ρₘᵢₙ)
                self.weighted_vruntime_offsets -=
                    resched_weight * (resched_vruntime - min_queued_vruntime);
            }
        } else {
            // This is not a reschedule. `weighted_vruntime_offsets` doesn't change.
        }

        self.current_task_data = Some(TaskData {
            task: task.clone(),
            id,
            vruntime,
            weight,
            vdeadline,
            is_exiting,
        });

        self.queue_len -= 1;
        self.queued_weight -= weight;

        Some(task)
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        _attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        // Φ' = ∑{i ∈ S}{wᵢ(ρᵢ' - ρₘᵢₙ')}
        //    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ')} + wₜ(ρₜ + Δ - ρₘᵢₙ')
        //    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ + ρₘᵢₙ - ρₘᵢₙ')} + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
        //    = ∑{i ∈ S\⦃t⦄}{wᵢ(ρᵢ - ρₘᵢₙ)} + (ρₘᵢₙ - ρₘᵢₙ')∑{i ∈ S\⦃t⦄}{wᵢ} + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
        //    = Φ - wₜ(ρₜ - ρₘᵢₙ) + (ρₘᵢₙ - ρₘᵢₙ')(W - wₜ) + wₜ(ρₜ - ρₘᵢₙ') + wₜΔ
        //    = Φ + wₜΔ - W(ρₘᵢₙ' - ρₘᵢₙ)

        // The data for the current task must have been set in `pick_next`.
        let current_task_data = self.current_task_data.as_mut().unwrap();

        let weight = current_task_data.weight;
        let vdelta = wall_to_virtual(rt.delta as i64, weight);
        let vdeadline = current_task_data.vdeadline;

        let old_vruntime = current_task_data.vruntime;
        let new_vruntime = old_vruntime + vdelta;
        current_task_data.vruntime = new_vruntime;

        // Adjust `weighted_vruntime_offsets`.
        let total_weight = weight + self.queued_weight;
        if let Some(min_queued_vruntime) = self.queue.min_vruntime() {
            // Advance `weighted_vruntime_offsets` with the contribution of the current task.
            self.weighted_vruntime_offsets += weight * vdelta;

            if old_vruntime < min_queued_vruntime {
                // The old task's vruntime was the minimum vruntime and now the
                // minimum vruntime is the minimum between the minimum queued
                // vruntime and the new task's vruntime.
                let new_min_vruntime = min_queued_vruntime.min(new_vruntime);
                self.weighted_vruntime_offsets -= total_weight * (new_min_vruntime - old_vruntime);
            }
        } else {
            // The queue is empty so the current task is the only one at play.
            // `weighted_vruntime_offsets` is zero and doesn't change because the
            // vruntime offset for the (only) task doesn't change, since it's also
            // the task with the minimum vruntime.
            debug_assert_eq!(self.weighted_vruntime_offsets, 0);

            // Also, since there's no competing task, we must return `false`.
            return false;
        }

        // At this point, the queue is guaranteed to be non-empty.
        match flags {
            UpdateFlags::Tick | UpdateFlags::Yield => new_vruntime >= vdeadline,
            UpdateFlags::Wait => true,
            UpdateFlags::Exit => {
                // Avoid computing and storing virtual lag in `pick_next`.
                current_task_data.is_exiting = true;
                true
            }
        }
    }

    fn len(&self) -> usize {
        self.queue_len
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
