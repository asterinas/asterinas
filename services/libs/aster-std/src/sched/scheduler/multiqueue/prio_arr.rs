use super::sched_entity::{PriorityOp, SchedEntity};
use alloc::sync::Arc;
use alloc::{collections::VecDeque, vec::Vec};
use aster_frame::task::{NeedResched, Priority, ReadPriority, TaskNumber, WakeUp};
use bitmaps::Bitmap;

const MIN_PRIORITY_VAL: u16 = Priority::highest().get();
const MAX_PRIORITY_VAL: u16 = Priority::lowest().get();
const NUM_PRIORITY: u16 = MAX_PRIORITY_VAL - MIN_PRIORITY_VAL + 1;

/// Task queues indexed by numeric priorities
pub struct PriorityArray<T: NeedResched + ReadPriority + WakeUp> {
    /// total number of tasks in the queue
    total_num: TaskNumber,

    /// array of task queues, one per priority, indexed by priority
    queues: Vec<VecDeque<Arc<SchedEntity<T>>>>,

    /// bitmap of non-empty queues
    bitmap: Bitmap<{ NUM_PRIORITY as usize }>,
}

impl<T: NeedResched + ReadPriority + WakeUp> PartialEq for PriorityArray<T> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}
impl<T: NeedResched + ReadPriority + WakeUp> Eq for PriorityArray<T> {}

impl<T: NeedResched + ReadPriority + WakeUp> Default for PriorityArray<T> {
    fn default() -> Self {
        let queues: Vec<_> = (MIN_PRIORITY_VAL..(MAX_PRIORITY_VAL + 1))
            .map(|_| VecDeque::new())
            .collect();
        debug_assert!(queues.len() == NUM_PRIORITY.into());
        Self {
            total_num: 0,
            queues,
            bitmap: Bitmap::new(),
        }
    }
}

impl<T: NeedResched + ReadPriority + WakeUp> PriorityArray<T> {
    #[inline]
    pub fn total_num(&self) -> TaskNumber {
        self.total_num
    }

    #[inline]
    pub fn empty(&self) -> bool {
        self.total_num == 0
    }

    /// If the queue indexed by `prio` is empty
    #[inline]
    pub fn is_empty_in(&self, prio: &Priority) -> bool {
        !self.bitmap.get(Self::prio_to_idx(prio))
    }

    #[inline]
    pub fn highest_prio(&self) -> Option<Priority> {
        (!self.empty()).then(|| Priority::new(self.bitmap.first_index().unwrap() as u16))
    }

    /// Get the index of the `queues` from the priority in the given entity
    #[inline]
    fn to_idx(entity: &Arc<SchedEntity<T>>) -> usize {
        Self::prio_to_idx(&entity.dyn_prio())
    }

    #[inline]
    fn prio_to_idx(prio: &Priority) -> usize {
        prio.get() as usize
    }

    /// Adding a task to this priority array
    pub fn enqueue(&mut self, entity: Arc<SchedEntity<T>>) {
        let idx = Self::to_idx(&entity);
        self.queues[idx].push_back(entity);
        self.set_bitmap_if_queue_not_empty(idx);
        self.inc_total_num();
    }

    #[inline]
    fn inc_total_num(&mut self) {
        self.total_num = self.total_num.checked_add(1).expect("task number overflow");
    }

    #[inline]
    fn dec_total_num(&mut self) {
        assert!(self.total_num > 0);
        self.total_num -= 1;
    }

    /// Removing a task from this priority array
    /// Returns true if the task is found and removed, false otherwise
    pub fn dequeue(&mut self, entity: &Arc<SchedEntity<T>>) -> bool {
        let idx = Self::to_idx(entity);
        let found = self.remove(entity);
        if found {
            self.dec_total_num();
            self.unset_bitmap_if_queue_empty(idx);
        }
        found
    }

    #[inline]
    fn unset_bitmap_if_queue_empty(&mut self, idx: usize) {
        if self.queues[idx].is_empty() {
            self.bitmap.set(idx, false);
        }
    }

    #[inline]
    fn set_bitmap_if_queue_not_empty(&mut self, idx: usize) {
        if !self.queues[idx].is_empty() {
            self.bitmap.set(idx, true);
        }
    }

    /// Remove a task from the queue
    /// Returns true if the task is found and removed, false otherwise
    fn remove(&mut self, entity: &Arc<SchedEntity<T>>) -> bool {
        let queue = &mut self.queues[Self::to_idx(entity)];
        if queue.is_empty() {
            return false;
        }

        let Some(target_idx) = queue.iter().position(|e| e == entity) else {
            return false;
        };
        queue.remove(target_idx).is_some()
    }

    /// Pick the next task to run from the active queues in a Round-Robin manner
    pub fn pick_next(&mut self) -> Option<Arc<SchedEntity<T>>> {
        self.bitmap.first_index().and_then(|idx| {
            let next = self.queues[idx].pop_front();
            self.dec_total_num();
            self.unset_bitmap_if_queue_empty(idx);
            next
        })
    }

    pub fn contains(&self, entity: &Arc<SchedEntity<T>>) -> bool {
        self.queues[Self::to_idx(entity)].contains(entity)
    }
}

#[if_cfg_ktest]
mod tests {
    use super::*;
    use crate::sched::scheduler::multiqueue::test::{new_mock_sched_entity, MockTask};

    fn default_prio_arr() -> PriorityArray<MockTask> {
        PriorityArray::default()
    }

    #[ktest]
    fn empty() {
        let priority_array = default_prio_arr();
        assert_eq!(priority_array.total_num(), 0);
        assert!(priority_array.empty());
        assert!(priority_array.highest_prio().is_none());
        for prio_val in Priority::highest().get()..=Priority::lowest().get() {
            let prio = Priority::new(prio_val);
            assert!(priority_array.is_empty_in(&prio));
        }
    }

    #[ktest]
    fn enqueue_basic() {
        let mut priority_array = default_prio_arr();
        let prio = Priority::new(101);
        let entity = new_mock_sched_entity("test", prio.clone());
        priority_array.enqueue(entity.clone());
        assert_eq!(priority_array.total_num(), 1);
        assert!(!priority_array.empty());
        assert!(!priority_array.is_empty_in(&prio));
        assert!(priority_array.contains(&entity));
        assert!(priority_array.highest_prio().is_some_and(|p| p == prio));
    }

    #[ktest]
    fn enqueue_multiple_task() {
        let mut priority_array = default_prio_arr();

        let prio_1 = Priority::new(101);
        let entity_1 = new_mock_sched_entity("test", prio_1.clone());
        priority_array.enqueue(entity_1.clone());

        let prio_2 = Priority::new(105);
        let entity_2 = new_mock_sched_entity("test", prio_2.clone());
        priority_array.enqueue(entity_2.clone());

        assert_eq!(priority_array.total_num(), 2);
        assert!(!priority_array.empty());
        assert!(!priority_array.is_empty_in(&prio_1) && !priority_array.is_empty_in(&prio_2));
        assert!(priority_array.contains(&entity_1) && priority_array.contains(&entity_2));
        assert!(priority_array.highest_prio().is_some_and(|p| p == prio_1));
    }

    #[ktest]
    fn dequeue_basic() {
        let mut priority_array = default_prio_arr();
        let prio = Priority::new(101);
        let entity = new_mock_sched_entity("test", prio.clone());
        priority_array.enqueue(entity.clone());

        assert!(priority_array.dequeue(&entity));
        assert_eq!(priority_array.total_num(), 0);
        assert!(priority_array.empty());
        assert!(priority_array.is_empty_in(&prio));

        // dequeue on empty
        assert!(!priority_array.contains(&entity));
        assert!(!priority_array.dequeue(&entity));
    }

    #[ktest]
    fn pick_next_basic() {
        let mut priority_array = default_prio_arr();
        let entity = new_mock_sched_entity("test", Priority::normal());

        // pick_next on empty PriorityArray
        assert!(priority_array.pick_next().is_none());

        // enqueue and pick_next
        priority_array.enqueue(entity.clone());
        assert_eq!(priority_array.total_num(), 1);
        assert!(priority_array
            .pick_next()
            .is_some_and(|picked| picked == entity));
        assert_eq!(priority_array.total_num(), 0);
    }

    mod pick_next_order {
        use super::*;

        #[ktest]
        fn sequential() {
            let mut priority_array = default_prio_arr();
            let entity_1 = new_mock_sched_entity("test_1", Priority::new(101));
            let entity_2 = new_mock_sched_entity("test_2", Priority::new(102));
            let entity_3 = new_mock_sched_entity("test_3", Priority::new(103));

            priority_array.enqueue(entity_1.clone());
            priority_array.enqueue(entity_2.clone());
            priority_array.enqueue(entity_3.clone());

            assert_eq!(priority_array.total_num(), 3);
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_1));
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_2));
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_3));
            assert!(priority_array.empty());
        }

        #[ktest]
        fn same_prio_fifo() {
            let mut priority_array = default_prio_arr();
            let entity_1 = new_mock_sched_entity("test_1", Priority::new(101));
            let entity_2 = new_mock_sched_entity("test_2", Priority::new(105));
            let entity_3 = new_mock_sched_entity("test_3", Priority::new(101));

            priority_array.enqueue(entity_1.clone());
            priority_array.enqueue(entity_2.clone());
            priority_array.enqueue(entity_3.clone());

            assert_eq!(priority_array.total_num(), 3);
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_1));
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_3));
            assert!(priority_array
                .pick_next()
                .is_some_and(|picked| picked == entity_2));
            assert!(priority_array.empty());
        }
    }
}
