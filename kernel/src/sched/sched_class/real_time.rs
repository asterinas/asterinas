// SPDX-License-Identifier: MPL-2.0

use alloc::collections::vec_deque::VecDeque;
use core::{array, num::NonZero};

use bitvec::{bitarr, BitArr};

use super::{time::base_slice_clocks, *};
use crate::sched::priority::Priority;

#[derive(Debug, Clone, Copy)]
pub struct RealTimeEntity {
    priority: Priority,
    time_slice: Option<NonZero<u64>>, // SCHED_RR; SCHED_FIFO

    start: u64,
}

impl RealTimeEntity {
    pub fn new(priority: Priority) -> Self {
        let n = base_slice_clocks() * 20; // 0.75ms * 20 = 15ms
        RealTimeEntity {
            priority,
            time_slice: NonZero::new(n),
            start: sched_clock(),
        }
    }
}

struct PrioArray {
    map: BitArr![for 100],
    queue: [VecDeque<Arc<Thread>>; 100],
}

impl core::fmt::Debug for PrioArray {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "map: ")?;
        f.debug_list().entries(self.map.iter_ones()).finish()?;
        for thread in self.queue.iter().flatten() {
            let entity = match *thread.sched_entity().lock() {
                SchedEntity::RealTime(real_time_entity) => real_time_entity,
                _ => unreachable!(),
            };
            writeln!(f, "    {entity:?}")?;
        }
        Ok(())
    }
}

impl PrioArray {
    fn queue_mut(&mut self, prio: u8) -> &mut VecDeque<Arc<Thread>> {
        &mut self.queue[usize::from(prio)]
    }

    fn enqueue(&mut self, thread: Arc<Thread>, prio: u8) {
        let queue = self.queue_mut(prio);
        let is_empty = queue.is_empty();
        queue.push_back(thread);
        if is_empty {
            self.map.set(usize::from(prio), true);
        }
    }

    fn pop_at(&mut self, prio: u8) -> Option<Arc<Thread>> {
        let queue = self.queue_mut(prio);
        let thread = queue.pop_front()?;
        if queue.is_empty() {
            self.map.set(usize::from(prio), false);
        }
        Some(thread)
    }

    fn pop(&mut self) -> Option<Arc<Thread>> {
        let prio = self.map.iter_ones().next()?;
        self.pop_at(prio as u8)
    }
}

#[derive(Debug)]
pub(super) struct RealTimeClassRq {
    index: bool,
    array: [PrioArray; 2],
}

impl RealTimeClassRq {
    fn active_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(self.index)]
    }

    fn swap_arrays(&mut self) {
        self.index = !self.index;
    }

    fn next_array(&mut self) -> &mut PrioArray {
        &mut self.array[usize::from(!self.index)]
    }
}

impl SchedClassRq for RealTimeClassRq {
    type Entity = RealTimeEntity;

    fn enqueue(&mut self, thread: Arc<Thread>) {
        let prio = match &*thread.sched_entity().lock() {
            SchedEntity::RealTime(entity) => entity.priority,
            _ => unreachable!(),
        };
        self.next_array().enqueue(thread, prio.range().get());
    }

    fn dequeue(&mut self, _: &RealTimeEntity) {}

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        self.active_array().pop().or_else(|| {
            self.swap_arrays();
            self.active_array().pop()
        })
    }

    fn update_current(&mut self, rt: &mut RealTimeEntity, flags: UpdateFlags) -> bool {
        let now = sched_clock();
        let should_preempt = match flags {
            UpdateFlags::Tick | UpdateFlags::Wait => match rt.time_slice {
                Some(ts) => ts.get() <= now - rt.start,
                None => false,
            },
            UpdateFlags::Yield => true,
        };
        if should_preempt {
            rt.start = now;
        }
        should_preempt
    }
}

pub fn new_class(_cpu: u32) -> RealTimeClassRq {
    RealTimeClassRq {
        index: false,
        array: array::from_fn(|_| PrioArray {
            map: bitarr![0; 100],
            queue: array::from_fn(|_| VecDeque::new()),
        }),
    }
}
