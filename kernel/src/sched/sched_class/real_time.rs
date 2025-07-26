// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Arc};
use core::{
    array,
    num::NonZero,
    sync::atomic::{AtomicU64, AtomicU8, Ordering::Relaxed},
};

use bitvec::{bitarr, BitArr};
use ostd::{
    cpu::CpuId,
    task::{
        scheduler::{EnqueueFlags, UpdateFlags},
        Task,
    },
};

use super::{time::base_slice_clocks, CurrentRuntime, SchedAttr, SchedClassRq};
use crate::{sched::nice::RangedU8, thread::AsThread};

pub type RealTimePriority = RangedU8<1, 99>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RealTimePolicy {
    Fifo,
    RoundRobin {
        base_slice_factor: Option<NonZero<u32>>,
    },
}

impl Default for RealTimePolicy {
    fn default() -> Self {
        Self::RoundRobin {
            base_slice_factor: None,
        }
    }
}

impl RealTimePolicy {
    fn to_time_slice(self) -> u64 {
        match self {
            RealTimePolicy::RoundRobin { base_slice_factor } => {
                base_slice_clocks()
                    * base_slice_factor
                        .map_or(DEFAULT_BASE_SLICE_FACTOR, |factor| u64::from(factor.get()))
            }
            RealTimePolicy::Fifo => 0,
        }
    }
}

/// The scheduling attribute for the REAL-TIME scheduling class.
///
/// This structure provides not-only the priority of the thread,
/// but also the time slice for the thread, measured in [`sched_clock`]s.
///
/// - If the time slice is not set, the thread is considered to be a FIFO
///   thread, and will be executed to its end if there no thread with a
///   lower priority.
/// - If the time slice is set, the thread is considered to be an RR
///   (round-robin) thread, and will be executed for the time slice, and
///   then it will be put back to the inactive array.
#[derive(Debug)]
pub struct RealTimeAttr {
    prio: AtomicU8,
    time_slice: AtomicU64, // 0 for SCHED_FIFO; other for SCHED_RR
}

const DEFAULT_BASE_SLICE_FACTOR: u64 = 20;

impl RealTimeAttr {
    pub fn new(prio: u8, policy: RealTimePolicy) -> Self {
        RealTimeAttr {
            prio: prio.into(),
            time_slice: AtomicU64::new(policy.to_time_slice()),
        }
    }

    pub fn update(&self, prio: u8, policy: RealTimePolicy) {
        self.prio.store(prio, Relaxed);
        self.time_slice.store(policy.to_time_slice(), Relaxed);
    }
}

struct PrioArray {
    map: BitArr![for 100],
    queue: [VecDeque<Arc<Task>>; 100],
}

impl core::fmt::Debug for PrioArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PrioArray")
            .field_with("map", |f| {
                f.debug_list().entries(self.map.iter_ones()).finish()
            })
            .field_with("queue", |f| {
                f.debug_list()
                    .entries(
                        (self.queue.iter().flatten())
                            .map(|task| task.as_thread().unwrap().sched_attr()),
                    )
                    .finish()
            })
            .finish()
    }
}

impl PrioArray {
    fn enqueue(&mut self, thread: Arc<Task>, prio: u8) {
        let queue = &mut self.queue[usize::from(prio)];
        let is_empty = queue.is_empty();
        queue.push_back(thread);
        if is_empty {
            self.map.set(usize::from(prio), true);
        }
    }

    fn pop(&mut self) -> Option<Arc<Task>> {
        let mut iter = self.map.iter_ones();
        let prio = iter.next()? as u8;

        let queue = &mut self.queue[usize::from(prio)];
        let thread = queue.pop_front()?;

        if queue.is_empty() {
            self.map.set(usize::from(prio), false);
        }
        Some(thread)
    }
}

/// The per-cpu run queue for the REAL-TIME scheduling class.
///
/// The REAL-TIME scheduling class is implemented as a classic O(1)
/// priority algorithm.
///
/// It uses a bit array to track which priority levels have runnable
/// threads, and a vector of queues to store the threads.
///
/// Threads are popped & dequeued from the active array (`array[index]`), and
/// are enqueued into the inactive array (`array[!index]`). When the active array
/// is empty, the 2 arrays are swapped by `index`.
#[derive(Debug)]
pub(super) struct RealTimeClassRq {
    #[expect(unused)]
    cpu: CpuId,
    index: bool,
    array: [PrioArray; 2],
    nr_running: usize,
}

impl RealTimeClassRq {
    pub fn new(cpu: CpuId) -> RealTimeClassRq {
        RealTimeClassRq {
            cpu,
            index: false,
            array: array::from_fn(|_| PrioArray {
                map: bitarr![0; 100],
                queue: array::from_fn(|_| VecDeque::new()),
            }),
            nr_running: 0,
        }
    }

    fn active_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(self.index)]
    }

    fn inactive_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(!self.index)]
    }

    fn swap_arrays(&mut self) {
        self.index = !self.index;
    }
}

impl SchedClassRq for RealTimeClassRq {
    fn enqueue(&mut self, entity: Arc<Task>, _: Option<EnqueueFlags>) {
        let sched_attr = entity.as_thread().unwrap().sched_attr();
        let prio = sched_attr.real_time.prio.load(Relaxed);
        self.inactive_array().enqueue(entity, prio);
        self.nr_running += 1;
    }

    fn len(&self) -> usize {
        self.nr_running
    }

    fn is_empty(&self) -> bool {
        self.nr_running == 0
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        if self.nr_running == 0 {
            return None;
        }

        (self.active_array().pop())
            .or_else(|| {
                self.swap_arrays();
                self.active_array().pop()
            })
            .inspect(|_| self.nr_running -= 1)
    }

    fn update_current(
        &mut self,
        rt: &CurrentRuntime,
        attr: &SchedAttr,
        flags: UpdateFlags,
    ) -> bool {
        let attr = &attr.real_time;

        match flags {
            UpdateFlags::Tick | UpdateFlags::Wait => match attr.time_slice.load(Relaxed) {
                0 => (self.inactive_array().map.iter_ones().next())
                    .is_some_and(|prio| prio > usize::from(attr.prio.load(Relaxed))),
                ts => ts <= rt.period_delta,
            },
            UpdateFlags::Yield => true,
            UpdateFlags::Exit => {
                // TODO: consider do more (e.g., time accounting)
                true
            }
        }
    }
}
