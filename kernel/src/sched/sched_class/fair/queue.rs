// SPDX-License-Identifier: MPL-2.0

//! This module implements the [`EligibilityQueue`] used in the EEVDF scheduler.

use alloc::boxed::Box;
use core::{cmp, mem};

/// Eligibility check: (ρ - ρₘᵢₙ)W ≤ Φ
fn is_eligible(
    vruntime: i64,
    min_vruntime: i64,
    total_weight: i64,
    weighted_vruntime_offsets: i64,
) -> bool {
    (vruntime - min_vruntime) * total_weight <= weighted_vruntime_offsets
}

/// A task augmented with scheduler-related data, meant for inhabiting the
/// [`EligibilityQueue`].
#[derive(Debug)]
pub(super) struct TaskData<T> {
    /// The task being scheduled.
    pub(super) task: T,
    /// A tie-breaking ID to avoid conflicts between tasks with the same virtual
    /// deadline.
    pub(super) id: u64,
    /// The virtual deadline of the scheduled task.
    pub(super) vdeadline: i64,
    /// The task weight, obtained from its nice value.
    pub(super) weight: i64,
    /// The accumulated vruntime of the task.
    pub(super) vruntime: i64,
    /// This flag allows a slight optimization to avoid bookkeeping lag when the
    /// task being rescheduled is exiting.
    pub(super) is_exiting: bool,
}

impl<T> Ord for TaskData<T> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        match self.vdeadline.cmp(&other.vdeadline) {
            cmp::Ordering::Equal => self.id.cmp(&other.id),
            ord => ord,
        }
    }
}
impl<T> PartialOrd for TaskData<T> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> PartialEq for TaskData<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}
impl<T> Eq for TaskData<T> {}

/// The [`EligibilityQueue`] is a balanced binary search tree in which tasks are
/// ordered by their virtual deadlines.
///
/// This data structure currently behaves as an AVL tree but an RB tree is likely
/// better.
pub(super) enum EligibilityQueue<T> {
    Node {
        data: TaskData<T>,
        min_vruntime: i64,
        height: i8,
        left: Box<Self>,
        right: Box<Self>,
    },
    Leaf,
}

impl<T> core::fmt::Debug for EligibilityQueue<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Leaf => write!(f, "Tree::Leaf"),
            Self::Node { data, .. } => write!(f, "Tree::Node[id={}]", data.id),
        }
    }
}

impl<T> EligibilityQueue<T> {
    /// Returns an empty queue.
    pub(super) fn new() -> Self {
        Self::Leaf
    }

    /// Whether the queue is empty or not.
    pub(super) fn is_empty(&self) -> bool {
        self.is_leaf()
    }

    /// Pushes a new [`TaskData`] to the queue.
    pub(super) fn push(&mut self, new: TaskData<T>) {
        match self {
            Self::Leaf => {
                let min_vruntime = new.vruntime;
                *self = Self::Node {
                    data: new,
                    min_vruntime,
                    height: 0,
                    left: Box::new(Self::new()),
                    right: Box::new(Self::new()),
                }
            }
            Self::Node {
                data, left, right, ..
            } => {
                match new.cmp(data) {
                    cmp::Ordering::Equal => *data = new,
                    cmp::Ordering::Less => left.push(new),
                    cmp::Ordering::Greater => right.push(new),
                }
                self.update_and_rebalance();
            }
        }
    }

    /// Returns the minimum vruntime in the queue, if any.
    pub(super) fn min_vruntime(&self) -> Option<i64> {
        match self {
            Self::Leaf => None,
            Self::Node { min_vruntime, .. } => Some(*min_vruntime),
        }
    }

    /// Pops the task with the earliest virtual deadline among the eligible tasks.
    /// If no task is eligible, pops the one with the earliest virtual deadline,
    /// ignoring eligibility.
    pub(super) fn pop(
        &mut self,
        global_min_vruntime: i64,
        total_weight: i64,
        weighted_vruntime_offsets: i64,
    ) -> Option<TaskData<T>> {
        match self {
            Self::Leaf => None,
            Self::Node {
                data, left, right, ..
            } => {
                if left.has_eligible_task(
                    global_min_vruntime,
                    total_weight,
                    weighted_vruntime_offsets,
                ) {
                    let res =
                        left.pop(global_min_vruntime, total_weight, weighted_vruntime_offsets);
                    if res.is_some() {
                        self.update_and_rebalance();
                    }
                    return res;
                }

                if is_eligible(
                    data.vruntime,
                    global_min_vruntime,
                    total_weight,
                    weighted_vruntime_offsets,
                ) {
                    // Take ownership of this node so we can move its children around.
                    let node = mem::replace(self, Self::Leaf);
                    let Self::Node {
                        data: task_data,
                        left,
                        mut right,
                        ..
                    } = node
                    else {
                        unreachable!();
                    };

                    // Case: no left child -> replace this node with right subtree.
                    if left.is_leaf() {
                        *self = *right;
                        if !self.is_leaf() {
                            self.update_and_rebalance();
                        }
                        return Some(task_data);
                    }

                    // Case: no right child -> replace this node with left subtree.
                    if right.is_leaf() {
                        *self = *left;
                        if !self.is_leaf() {
                            self.update_and_rebalance();
                        }
                        return Some(task_data);
                    }

                    // Case: two children -> replace with in-order successor (min of right).
                    let successor = right.pop_min().unwrap();
                    *self = Self::Node {
                        data: successor,
                        min_vruntime: 0, // Fixed by `update_and_rebalance`.
                        height: 0,
                        left,
                        right,
                    };
                    self.update_and_rebalance();
                    return Some(task_data);
                }

                if right.has_eligible_task(
                    global_min_vruntime,
                    total_weight,
                    weighted_vruntime_offsets,
                ) {
                    let res =
                        right.pop(global_min_vruntime, total_weight, weighted_vruntime_offsets);
                    if res.is_some() {
                        self.update_and_rebalance();
                    }
                    return res;
                }

                let res = self.pop_min();
                if res.is_some() {
                    self.update_and_rebalance();
                }
                res
            }
        }
    }

    /// Returns the minimum between the given `vruntime` and the minimum vruntime
    /// in the queue. If the queue is empty, just returns `vruntime`.
    pub(super) fn min_vruntime_against(&self, vruntime: i64) -> i64 {
        match self {
            Self::Leaf => vruntime,
            Self::Node { min_vruntime, .. } => vruntime.min(*min_vruntime),
        }
    }

    fn has_eligible_task(
        &self,
        global_min_vruntime: i64,
        total_weight: i64,
        weighted_vruntime_offsets: i64,
    ) -> bool {
        match self {
            Self::Leaf => false,
            Self::Node { min_vruntime, .. } => is_eligible(
                *min_vruntime,
                global_min_vruntime,
                total_weight,
                weighted_vruntime_offsets,
            ),
        }
    }

    fn pop_min(&mut self) -> Option<TaskData<T>> {
        match self {
            Self::Leaf => None,
            Self::Node { left, .. } => {
                if left.is_leaf() {
                    let old_self = mem::replace(self, Self::Leaf);
                    if let Self::Node { data, right, .. } = old_self {
                        *self = *right;
                        return Some(data);
                    }
                    None
                } else {
                    let result = left.pop_min();
                    self.update_and_rebalance();
                    result
                }
            }
        }
    }

    fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf)
    }

    fn update(&mut self) {
        if let Self::Node {
            data,
            height,
            min_vruntime,
            left,
            right,
        } = self
        {
            *height = 1 + left.height().max(right.height());
            *min_vruntime = right.min_vruntime_against(left.min_vruntime_against(data.vruntime));
        }
    }

    fn height(&self) -> i8 {
        match self {
            Self::Leaf => -1,
            Self::Node { height, .. } => *height,
        }
    }

    fn balance_factor(&self) -> i8 {
        match self {
            Self::Leaf => 0,
            Self::Node { left, right, .. } => left.height() - right.height(),
        }
    }

    fn rotate_left(&mut self) {
        if let Self::Node { right, .. } = self {
            let right_node = mem::replace(right, Box::new(Self::Leaf));
            if let Self::Node {
                data: rdata,
                min_vruntime: rmin_vruntime,
                height: rheight,
                left: rleft,
                right: rright,
            } = *right_node
            {
                let old = mem::replace(self, Self::Leaf);
                if let Self::Node {
                    data,
                    min_vruntime,
                    height,
                    left,
                    right: _,
                } = old
                {
                    *self = Self::Node {
                        data: rdata,
                        min_vruntime: rmin_vruntime,
                        height: rheight,
                        left: Box::new(Self::Node {
                            data,
                            min_vruntime,
                            height,
                            left,
                            right: rleft,
                        }),
                        right: rright,
                    };
                }
            }
        }
        if let Self::Node { left, right, .. } = self {
            left.update();
            right.update();
        }
        self.update();
    }

    fn rotate_right(&mut self) {
        if let Self::Node { left, .. } = self {
            let left_node = mem::replace(left, Box::new(Self::Leaf));
            if let Self::Node {
                data: ldata,
                min_vruntime: lmin_vruntime,
                height: lheight,
                left: lleft,
                right: lright,
            } = *left_node
            {
                let old = mem::replace(self, Self::Leaf);
                if let Self::Node {
                    data,
                    min_vruntime,
                    height,
                    left: _,
                    right,
                } = old
                {
                    *self = Self::Node {
                        data: ldata,
                        min_vruntime: lmin_vruntime,
                        height: lheight,
                        left: lleft,
                        right: Box::new(Self::Node {
                            data,
                            min_vruntime,
                            height,
                            left: lright,
                            right,
                        }),
                    };
                }
            }
        }
        if let Self::Node { left, right, .. } = self {
            left.update();
            right.update();
        }
        self.update();
    }

    fn update_and_rebalance(&mut self) {
        self.update();

        let bf = self.balance_factor();
        if bf > 1 {
            let Self::Node { left, .. } = self else {
                return;
            };
            if left.balance_factor() < 0 {
                left.rotate_left();
            }
            self.rotate_right();
        } else if bf < -1 {
            let Self::Node { right, .. } = self else {
                return;
            };
            if right.balance_factor() > 0 {
                right.rotate_right();
            }
            self.rotate_left();
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    // Helper function to create a simple task
    fn create_task(id: u64, vdeadline: i64, weight: i64, vruntime: i64) -> TaskData<u64> {
        TaskData {
            task: id,
            id,
            vdeadline,
            weight,
            vruntime,
            is_exiting: false,
        }
    }

    // Helper to verify tree invariants
    fn verify_tree_invariants<T>(queue: &EligibilityQueue<T>) -> (i8, i64) {
        match queue {
            EligibilityQueue::Leaf => (-1, i64::MAX),
            EligibilityQueue::Node {
                data,
                min_vruntime,
                height,
                left,
                right,
            } => {
                let (left_height, left_min) = verify_tree_invariants(left);
                let (right_height, right_min) = verify_tree_invariants(right);

                // Check height calculation
                let expected_height = 1 + left_height.max(right_height);
                assert_eq!(*height, expected_height, "Height calculation incorrect");

                // Check min_vruntime calculation
                let expected_min = data.vruntime.min(left_min).min(right_min);
                assert_eq!(
                    *min_vruntime, expected_min,
                    "min_vruntime calculation incorrect"
                );

                // Check balance factor
                let balance_factor = left_height - right_height;
                assert!(
                    balance_factor.abs() <= 1,
                    "Tree is not balanced, balance factor: {}",
                    balance_factor
                );

                (*height, *min_vruntime)
            }
        }
    }

    #[ktest]
    fn test_empty_tree() {
        let queue: EligibilityQueue<&str> = EligibilityQueue::new();
        assert!(queue.is_empty());
        assert!(queue.min_vruntime().is_none());

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_single_node() {
        let mut queue = EligibilityQueue::new();
        let task = create_task(1, 100, 10, 50);

        queue.push(task);
        assert!(!queue.is_empty());
        assert_eq!(queue.min_vruntime(), Some(50));

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_multiple_nodes_ordered_insertion() {
        let mut queue = EligibilityQueue::new();

        // Insert in increasing order of vdeadline
        for i in 1..=10i64 {
            queue.push(create_task(i as u64, i * 10, 10, i * 5));
        }

        assert!(!queue.is_empty());
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_multiple_nodes_reverse_ordered_insertion() {
        let mut queue = EligibilityQueue::new();

        // Insert in decreasing order of vdeadline
        for i in (1..=10i64).rev() {
            queue.push(create_task(i as u64, i * 10, 10, i * 5));
        }

        assert!(!queue.is_empty());
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_multiple_nodes_random_insertion() {
        let mut queue = EligibilityQueue::new();

        let tasks = [
            create_task(3, 30, 10, 15),
            create_task(1, 10, 10, 5),
            create_task(5, 50, 10, 25),
            create_task(2, 20, 10, 10),
            create_task(4, 40, 10, 20),
        ];

        for task in tasks {
            queue.push(task);
        }

        assert!(!queue.is_empty());
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_pop_min_basic() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(2, 20, 10, 10));
        queue.push(create_task(1, 10, 10, 5));
        queue.push(create_task(3, 30, 10, 15));

        let min_task = queue.pop_min();
        assert_eq!(min_task.unwrap().id, 1);

        verify_tree_invariants(&queue);

        let second_min = queue.pop_min();
        assert_eq!(second_min.unwrap().id, 2);

        verify_tree_invariants(&queue);

        let third_min = queue.pop_min();
        assert_eq!(third_min.unwrap().id, 3);

        assert!(queue.is_empty());
    }

    #[ktest]
    fn test_pop_eligible_all_eligible() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 5));
        queue.push(create_task(2, 20, 10, 10));
        queue.push(create_task(3, 30, 10, 15));

        // All tasks should be eligible with these parameters
        let task = queue.pop(0, 30, 1000); // Large offset makes all eligible
        assert_eq!(task.unwrap().id, 1); // Should return earliest vdeadline

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_pop_eligible_none_eligible() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 100));
        queue.push(create_task(2, 20, 10, 200));
        queue.push(create_task(3, 30, 10, 300));

        // No tasks should be eligible with these parameters
        let task = queue.pop(0, 30, 50); // Small offset makes none eligible
                                         // Should return the minimum vdeadline task regardless of eligibility
        assert_eq!(task.unwrap().id, 1);

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_pop_eligible_some_eligible() {
        let mut queue = EligibilityQueue::new();

        // Task 1: vruntime=50, eligible if (50-0)*total_weight <= 600
        queue.push(create_task(1, 10, 10, 50));
        // Task 2: vruntime=70, eligible if (70-0)*total_weight <= 600
        queue.push(create_task(2, 20, 10, 70));
        // Task 3: vruntime=30, eligible if (30-0)*total_weight <= 600
        queue.push(create_task(3, 30, 10, 30));

        // Let's assume total_weight = 30 (sum of all task weights: 10+10+10)
        let total_weight = 30;
        let weighted_vruntime_offsets = 600;

        // Eligibility check:
        // Task 1: (50-0)*30 = 1500 <= 600 ✗
        // Task 2: (70-0)*30 = 2100 <= 600 ✗
        // Task 3: (30-0)*30 = 900 <= 600 ✗
        // None are eligible, so should return earliest vdeadline (task 1)

        let task = queue.pop(0, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 1); // Earliest vdeadline

        verify_tree_invariants(&queue);

        // After removing task 1, total_weight = 20
        let total_weight = 20;

        // Task 2: (70-0)*20 = 1400 <= 600 ✗
        // Task 3: (30-0)*20 = 600 <= 600 ✓
        // Task 3 is eligible and has earlier vdeadline than task 2

        let task = queue.pop(0, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 3);

        verify_tree_invariants(&queue);

        // Last task (task 2) - only one left
        let total_weight = 10;
        let task = queue.pop(0, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 2);

        assert!(queue.is_empty());
    }

    #[ktest]
    fn test_pop_complex_eligibility() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 100));
        queue.push(create_task(2, 20, 10, 50));
        queue.push(create_task(3, 30, 10, 200));

        let total_weight = 30;
        let global_min_vruntime = 40;
        let weighted_vruntime_offsets = 700;

        // Eligibility calculations:
        // Task 1: (100-40)*30 = 1800 <= 700 ✗
        // Task 2: (50-40)*30 = 300 <= 700 ✓
        // Task 3: (200-40)*30 = 4800 <= 700 ✗

        // Only task 2 is eligible, so it should be returned
        let task = queue.pop(global_min_vruntime, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 2);

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_pop_eligible_with_different_weights() {
        let mut queue = EligibilityQueue::new();

        // Tasks with different weights
        let task1 = create_task(1, 10, 5, 50); // weight=5
        let task2 = create_task(2, 20, 15, 30); // weight=15
        let task3 = create_task(3, 30, 10, 70); // weight=10

        queue.push(task1);
        queue.push(task2);
        queue.push(task3);

        let total_weight = 30;
        let global_min_vruntime = 0;
        let weighted_vruntime_offsets = 1200;

        // Eligibility calculations:
        // Task 1: (50-0)*30 = 1500 <= 1200 ✗
        // Task 2: (30-0)*30 = 900 <= 1200 ✓
        // Task 3: (70-0)*30 = 2100 <= 1200 ✗

        // Task 2 is eligible and has earliest vdeadline among eligible
        let task = queue.pop(global_min_vruntime, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 2);

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_pop_changing_global_min_vruntime() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 100));
        queue.push(create_task(2, 20, 10, 150));
        queue.push(create_task(3, 30, 10, 200));

        let total_weight = 30;
        let weighted_vruntime_offsets = 1500;

        // With global_min_vruntime=0:
        // Task 1: (100-0)*30 = 3000 <= 1500 ✗
        // Task 2: (150-0)*30 = 4500 <= 1500 ✗
        // Task 3: (200-0)*30 = 6000 <= 1500 ✗
        // None eligible, return earliest (task 1)
        let task = queue.pop(0, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 1);

        // With global_min_vruntime=100 after some time:
        // Task 2: (150-100)*20 = 1000 <= 1500 ✓ (total_weight now 20)
        // Task 3: (200-100)*20 = 2000 <= 1500 ✗
        let total_weight = 20;
        let task = queue.pop(100, total_weight, weighted_vruntime_offsets);
        assert_eq!(task.unwrap().id, 2);

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_avl_rotation_cases() {
        // Test left rotation (right heavy)
        let mut queue = EligibilityQueue::new();
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(2, 20, 10, 20));
        queue.push(create_task(3, 30, 10, 30)); // Should trigger left rotation

        verify_tree_invariants(&queue);

        // Test right rotation (left heavy)
        let mut queue = EligibilityQueue::new();
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(2, 20, 10, 20));
        queue.push(create_task(1, 10, 10, 10)); // Should trigger right rotation

        verify_tree_invariants(&queue);

        // Test left-right rotation
        let mut queue = EligibilityQueue::new();
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(2, 20, 10, 20)); // Should trigger left-right rotation

        verify_tree_invariants(&queue);

        // Test right-left rotation
        let mut queue = EligibilityQueue::new();
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(2, 20, 10, 20)); // Should trigger right-left rotation

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_min_vruntime_calculation() {
        let mut queue = EligibilityQueue::new();

        assert_eq!(queue.min_vruntime(), None);

        queue.push(create_task(1, 10, 10, 50));
        assert_eq!(queue.min_vruntime(), Some(50));

        queue.push(create_task(2, 20, 10, 30)); // New min
        assert_eq!(queue.min_vruntime(), Some(30));

        queue.push(create_task(3, 30, 10, 40));
        assert_eq!(queue.min_vruntime(), Some(30));

        queue.push(create_task(4, 40, 10, 20)); // New min
        assert_eq!(queue.min_vruntime(), Some(20));

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_task_ordering() {
        let task1 = create_task(1, 100, 10, 50);
        let task2 = create_task(2, 100, 10, 50); // Same vdeadline, different ID
        let task3 = create_task(3, 200, 10, 50);

        assert!(task1 < task2); // ID break tie
        assert!(task2 < task3); // vdeadline comparison
        assert!(task1 < task3);

        // Test equality (based on ID only)
        let task1_clone = create_task(1, 100, 10, 50);
        assert_eq!(task1, task1_clone);
        assert_ne!(task1, task2);
    }

    #[ktest]
    fn test_stress_large_tree() {
        let mut queue = EligibilityQueue::new();
        const COUNT: usize = 1000;

        // Insert many tasks
        for i in 0..COUNT {
            queue.push(create_task(i as u64, (i * 10) as i64, 10, (i * 5) as i64));
        }

        verify_tree_invariants(&queue);

        // Pop all tasks and verify order
        let mut last_vdeadline = -1;
        for _ in 0..COUNT {
            let task = queue.pop_min().expect("Should have tasks remaining");
            assert!(task.vdeadline >= last_vdeadline);
            last_vdeadline = task.vdeadline;

            verify_tree_invariants(&queue);
        }

        assert!(queue.is_empty());
    }

    #[ktest]
    fn test_pop_with_global_min_vruntime_updates() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 100));
        queue.push(create_task(2, 20, 10, 50));
        queue.push(create_task(3, 30, 10, 150));

        // With increasing global_min_vruntime, eligibility changes
        let task1 = queue.pop(0, 10, 1000); // All eligible, get task 1
        assert_eq!(task1.unwrap().id, 1);

        let task2 = queue.pop(50, 10, 1000);
        // Now: task2 vruntime=50, (50-50)*10 = 0 <= 1000 ✓
        //      task3 vruntime=150, (150-50)*10 = 1000 <= 1000 ✓
        // Should get task2 (earlier vdeadline)
        assert_eq!(task2.unwrap().id, 2);

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_edge_case_vruntime_values() {
        let mut queue = EligibilityQueue::new();

        // Test with extreme vruntime values
        queue.push(create_task(1, 10, 10, i64::MIN));
        queue.push(create_task(2, 20, 10, i64::MAX));
        queue.push(create_task(3, 30, 10, 0));

        verify_tree_invariants(&queue);

        let min_vruntime = queue.min_vruntime();
        assert_eq!(min_vruntime, Some(i64::MIN));

        // Test pop with edge case parameters
        let _ = queue.pop(i64::MIN, 1, i64::MAX);
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn test_mixed_operations() {
        let mut queue = EligibilityQueue::new();

        // Push some tasks
        queue.push(create_task(1, 100, 10, 50));
        queue.push(create_task(2, 50, 10, 30));
        queue.push(create_task(3, 150, 10, 70));

        verify_tree_invariants(&queue);

        // Pop one
        let task = queue.pop(0, 10, 1000);
        assert_eq!(task.unwrap().id, 2); // Earliest vdeadline

        verify_tree_invariants(&queue);

        // Push more
        queue.push(create_task(4, 75, 10, 40));
        queue.push(create_task(5, 200, 10, 90));

        verify_tree_invariants(&queue);

        // Pop remaining in correct order
        let expected_order = [4, 1, 3, 5];
        for &expected_id in &expected_order {
            let task = queue.pop_min().expect("Should have task");
            assert_eq!(task.id, expected_id);
            if !queue.is_empty() {
                verify_tree_invariants(&queue);
            }
        }

        assert!(queue.is_empty());
    }
}
