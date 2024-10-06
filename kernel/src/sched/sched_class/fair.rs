// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, sync::Arc};
use core::{cmp, ops::Bound};

use ostd::{
    cpu::num_cpus,
    sync::{PreemptDisabled, SpinLockGuard},
    task::scheduler::UpdateFlags,
};

use super::{
    sched_clock,
    time::{base_slice_clocks, min_period_clocks},
    SchedClassRq, SchedEntity,
};
use crate::{
    sched::priority::{Nice, NiceRange},
    thread::Thread,
};

pub const fn nice_to_weight(nice: Nice) -> u64 {
    /// Calculated using the formula below:
    ///     
    ///     weight = 1024 * 1.1^(-nice)
    ///
    /// We propose that every increment of the nice value results
    /// in 10% change of the CPU load weight.
    const NICE_TO_WEIGHT: [u32; 40] = [
        88761, 71755, 56483, 46273, 36291, 29154, 23254, 18705, 14949, 11916, 9548, 7620, 6100,
        4904, 3906, 3121, 2501, 1991, 1586, 1277, 1024, 820, 655, 526, 423, 335, 272, 215, 172,
        137, 110, 87, 70, 56, 45, 36, 29, 23, 18, 15,
    ];
    NICE_TO_WEIGHT[(nice.range().get() + 20) as usize] as u64
}
const WEIGHT_0: u64 = nice_to_weight(Nice::new(NiceRange::new(0)));

#[derive(Debug, Clone, Copy)]
pub struct VRuntime {
    weight: u64,
    vruntime: u64,
    start: u64,
    period_start: u64,
}

impl VRuntime {
    pub fn new(nice: Nice) -> Self {
        let now = sched_clock();
        VRuntime {
            weight: nice_to_weight(nice),

            vruntime: 0,
            start: now,
            period_start: now,
        }
    }

    fn get_with_cur(&self, cur: u64) -> u64 {
        self.vruntime + ((cur - self.start) * WEIGHT_0 / self.weight)
    }

    fn get(&self) -> u64 {
        self.vruntime
    }

    fn tick(&mut self, load: u64, period: u64) -> bool {
        let cur = sched_clock();
        self.vruntime = self.get_with_cur(cur);
        self.start = cur;

        debug_assert!(load != 0);
        debug_assert!(period != 0);
        debug_assert!(cur - self.period_start != 0);

        let slice = period * self.weight;
        if cur - self.period_start > slice / load {
            self.period_start = cur;
            true
        } else {
            false
        }
    }
}

impl Ord for VRuntime {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        (self.get().cmp(&other.get())).then_with(|| self.start.cmp(&other.start))
    }
}

impl PartialOrd for VRuntime {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for VRuntime {}

impl PartialEq for VRuntime {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get() && self.start == other.start
    }
}

struct FairQueueItem(Arc<Thread>);

impl FairQueueItem {
    fn key(&self) -> VRuntime {
        match *self.0.sched_entity().lock() {
            SchedEntity::Fair(vruntime) => vruntime,
            _ => unreachable!(),
        }
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
        Some(self.key().cmp(&other.key()))
    }
}

impl Ord for FairQueueItem {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

pub(super) struct FairClassRq {
    cpu: u32,
    threads: BTreeMap<FairQueueItem, ()>,
    load: u64,
}

impl core::fmt::Debug for FairClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "Fair: load = {}, num = {}",
            self.load,
            self.threads.len() + 1
        )?;
        writeln!(f, "  threads: ")?;
        for thread in self.threads.keys() {
            let vr = thread.key();
            writeln!(f, "    {vr:?}")?;
        }
        Ok(())
    }
}

impl FairClassRq {
    fn new(cpu: u32) -> Self {
        Self {
            cpu,
            threads: BTreeMap::new(),
            load: 0,
        }
    }

    fn period(&self) -> u64 {
        let base_slice_clks = base_slice_clocks();
        let min_period_clks = min_period_clocks();

        let min_gran_clks = base_slice_clks * u64::from((1 + num_cpus()).ilog2());
        (min_gran_clks * (self.threads.len() + 1) as u64).max(min_period_clks)
    }

    fn pop(&mut self, target_cpu: u32) -> Option<Arc<Thread>> {
        let mut front = self.threads.lower_bound_mut(Bound::Unbounded);
        let FairQueueItem(thread) = loop {
            let (thread, _) = front.peek_next()?;
            if !thread.0.lock_cpu_affinity().contains(target_cpu) {
                front.next().unwrap();
                continue;
            }

            let (thread, _) = front.remove_next().unwrap();
            break thread;
        };

        match &mut *thread.sched_entity().lock() {
            SchedEntity::Fair(vruntime) => vruntime.start = sched_clock(),
            _ => unreachable!(),
        }

        Some(thread)
    }
}

impl SchedClassRq for FairClassRq {
    type Entity = VRuntime;

    fn enqueue(
        &mut self,
        thread: Arc<Thread>,
        entity: SpinLockGuard<'_, SchedEntity, PreemptDisabled>,
    ) {
        match &*entity {
            SchedEntity::Fair(vruntime) => self.load += vruntime.weight,
            _ => unreachable!(),
        };
        drop(entity);
        self.threads.insert(FairQueueItem(thread), ());
    }

    fn dequeue(&mut self, vruntime: &VRuntime) {
        self.load -= vruntime.weight;
    }

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        self.pop(self.cpu)
    }

    fn update_current(&mut self, vruntime: &mut VRuntime, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Yield => {
                vruntime.period_start = sched_clock();
                true
            }
            UpdateFlags::Tick | UpdateFlags::Wait => vruntime.tick(self.load, self.period()),
        }
    }
}

pub fn new_class(cpu: u32) -> FairClassRq {
    FairClassRq::new(cpu)
}
