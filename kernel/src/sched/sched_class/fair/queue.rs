// SPDX-License-Identifier: MPL-2.0

//! This module implements the [`EligibilityQueue`] used in the EEVDF scheduler.

use alloc::vec::Vec;
use core::cmp;

/// Eligibility check: (ρ - ρₘᵢₙ)W ≤ Φ
///
/// The subtraction is non-negative (callers guarantee `vruntime >= min_vruntime`).
/// The product fits in `i64` because lag clamping at reschedule time (see
/// `FairClassRq::pick_next`) constrains the per-CPU vruntime spread to
/// O(`lag_limit_clocks` · `WEIGHT_0` / `w_min`), and `total_weight` is bounded
/// by the per-CPU task count times `w_max`. With current constants
/// (`base_slice` = 0.7 ms, tick = 1 ms, `w_min` ≈ 15, `w_max` ≈ 89K) and 1000
/// tasks per CPU, the worst-case product is ~10¹⁷ — roughly 1% of `i64::MAX`.
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
/// `PartialEq` compares by `id` alone, while `Ord` compares by
/// `(vdeadline, id)`. This is consistent in practice because IDs are
/// assigned from a monotonic counter in `FairClassRq::enqueue`,
/// so equal IDs imply equal vdeadlines.
impl<T> PartialEq for TaskData<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}
impl<T> Eq for TaskData<T> {}

/// Sentinel value representing a null/absent node.
const NIL: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Color {
    Red,
    Black,
}

struct Node<T> {
    data: TaskData<T>,
    color: Color,
    parent: u32,
    left: u32,
    right: u32,
    min_vruntime: i64,
}

/// The [`EligibilityQueue`] is a red-black tree augmented with min-vruntime
/// tracking, in which tasks are ordered by their virtual deadlines.
///
/// All tree operations are iterative to avoid deep stack usage in the kernel.
pub(super) struct EligibilityQueue<T> {
    nodes: Vec<Option<Node<T>>>,
    root: u32,
    free_list: Vec<u32>,
    len: usize,
}

impl<T> core::fmt::Debug for EligibilityQueue<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.root == NIL {
            write!(f, "EligibilityQueue::Empty")
        } else {
            write!(
                f,
                "EligibilityQueue[root_id={}]",
                self.node(self.root).data.id
            )
        }
    }
}

impl<T> EligibilityQueue<T> {
    /// Returns an empty queue.
    pub(super) fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: NIL,
            free_list: Vec::new(),
            len: 0,
        }
    }

    /// Returns the number of tasks in the queue.
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// Pushes a new [`TaskData`] to the queue.
    pub(super) fn push(&mut self, new: TaskData<T>) {
        let mut parent = NIL;
        let mut curr = self.root;
        let mut go_left = true;

        while curr != NIL {
            parent = curr;
            match new.cmp(&self.node(curr).data) {
                cmp::Ordering::Equal => {
                    self.node_mut(curr).data = new;
                    self.update_min_vruntime_to_root(curr);
                    return;
                }
                cmp::Ordering::Less => {
                    go_left = true;
                    curr = self.node(curr).left;
                }
                cmp::Ordering::Greater => {
                    go_left = false;
                    curr = self.node(curr).right;
                }
            }
        }

        let z = self.alloc_node(new, Color::Red);
        self.node_mut(z).parent = parent;

        if parent == NIL {
            self.root = z;
        } else if go_left {
            self.node_mut(parent).left = z;
        } else {
            self.node_mut(parent).right = z;
        }

        self.update_min_vruntime_to_root(z);
        self.insert_fixup(z);
        self.len += 1;
    }

    /// Returns the minimum vruntime in the queue, if any.
    pub(super) fn min_vruntime(&self) -> Option<i64> {
        if self.root == NIL {
            None
        } else {
            Some(self.node(self.root).min_vruntime)
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
        if self.root == NIL {
            return None;
        }

        if is_eligible(
            self.node(self.root).min_vruntime,
            global_min_vruntime,
            total_weight,
            weighted_vruntime_offsets,
        ) {
            let idx = self.find_first_eligible(
                global_min_vruntime,
                total_weight,
                weighted_vruntime_offsets,
            );
            if idx != NIL {
                return Some(self.delete_node(idx));
            }
        }

        // Fallback: pop the task with the earliest virtual deadline.
        let min_idx = self.tree_minimum(self.root);
        Some(self.delete_node(min_idx))
    }

    /// Returns the minimum between the given `vruntime` and the minimum vruntime
    /// in the queue. If the queue is empty, just returns `vruntime`.
    pub(super) fn min_vruntime_against(&self, vruntime: i64) -> i64 {
        if self.root == NIL {
            vruntime
        } else {
            vruntime.min(self.node(self.root).min_vruntime)
        }
    }

    // -- High-level internal operations --

    /// Finds the first eligible node by in-order traversal (earliest vdeadline),
    /// using the augmented `min_vruntime` to prune ineligible subtrees.
    fn find_first_eligible(
        &self,
        global_min_vruntime: i64,
        total_weight: i64,
        weighted_vruntime_offsets: i64,
    ) -> u32 {
        let mut curr = self.root;
        while curr != NIL {
            let left = self.node(curr).left;
            if left != NIL
                && is_eligible(
                    self.node(left).min_vruntime,
                    global_min_vruntime,
                    total_weight,
                    weighted_vruntime_offsets,
                )
            {
                curr = left;
                continue;
            }

            if is_eligible(
                self.node(curr).data.vruntime,
                global_min_vruntime,
                total_weight,
                weighted_vruntime_offsets,
            ) {
                return curr;
            }

            let right = self.node(curr).right;
            if right != NIL
                && is_eligible(
                    self.node(right).min_vruntime,
                    global_min_vruntime,
                    total_weight,
                    weighted_vruntime_offsets,
                )
            {
                curr = right;
                continue;
            }

            break;
        }
        NIL
    }

    /// Removes node `z` from the tree and returns its data.
    fn delete_node(&mut self, z: u32) -> TaskData<T> {
        let zl = self.node(z).left;
        let zr = self.node(z).right;
        let z_color = self.node(z).color;

        let y_orig_color;
        let x;
        let x_parent;
        let mut y_moved = NIL;

        if zl == NIL {
            y_orig_color = z_color;
            x = zr;
            x_parent = self.node(z).parent;
            self.transplant(z, zr);
        } else if zr == NIL {
            y_orig_color = z_color;
            x = zl;
            x_parent = self.node(z).parent;
            self.transplant(z, zl);
        } else {
            // Two children: replace z with its in-order successor.
            let y = self.tree_minimum(zr);
            y_orig_color = self.node(y).color;
            x = self.node(y).right;

            if self.node(y).parent == z {
                x_parent = y;
            } else {
                x_parent = self.node(y).parent;
                self.transplant(y, x);
                self.node_mut(y).right = zr;
                self.node_mut(zr).parent = y;
            }

            self.transplant(z, y);
            self.node_mut(y).left = zl;
            self.node_mut(zl).parent = y;
            self.node_mut(y).color = z_color;
            y_moved = y;
        }

        if y_orig_color == Color::Black {
            self.delete_fixup(x, x_parent);
        }

        // Restore the augmented min_vruntime from all affected paths to root.
        if x_parent != NIL {
            self.update_min_vruntime_to_root(x_parent);
        }
        if y_moved != NIL {
            self.update_min_vruntime_to_root(y_moved);
        }

        self.len -= 1;
        self.free_node(z)
    }

    // -- Red-black tree balancing --

    fn insert_fixup(&mut self, mut z: u32) {
        while z != self.root && self.color_of(self.parent_of(z)) == Color::Red {
            let p = self.parent_of(z);
            let gp = self.parent_of(p);

            if p == self.left_of(gp) {
                let uncle = self.right_of(gp);
                if self.color_of(uncle) == Color::Red {
                    self.set_color(p, Color::Black);
                    self.set_color(uncle, Color::Black);
                    self.set_color(gp, Color::Red);
                    z = gp;
                } else {
                    if z == self.right_of(p) {
                        z = p;
                        self.rotate_left(z);
                    }
                    let p = self.parent_of(z);
                    let gp = self.parent_of(p);
                    self.set_color(p, Color::Black);
                    self.set_color(gp, Color::Red);
                    self.rotate_right(gp);
                }
            } else {
                let uncle = self.left_of(gp);
                if self.color_of(uncle) == Color::Red {
                    self.set_color(p, Color::Black);
                    self.set_color(uncle, Color::Black);
                    self.set_color(gp, Color::Red);
                    z = gp;
                } else {
                    if z == self.left_of(p) {
                        z = p;
                        self.rotate_right(z);
                    }
                    let p = self.parent_of(z);
                    let gp = self.parent_of(p);
                    self.set_color(p, Color::Black);
                    self.set_color(gp, Color::Red);
                    self.rotate_left(gp);
                }
            }
        }
        self.set_color(self.root, Color::Black);
    }

    fn delete_fixup(&mut self, mut x: u32, mut x_parent: u32) {
        while x != self.root && self.color_of(x) == Color::Black {
            let x_is_left = if x != NIL {
                x == self.node(x_parent).left
            } else {
                // When x is NIL, the sibling is always non-NIL (RB invariant),
                // so the NIL side is whichever child slot is empty.
                self.node(x_parent).left == NIL
            };

            if x_is_left {
                let mut w = self.node(x_parent).right;

                if self.color_of(w) == Color::Red {
                    self.set_color(w, Color::Black);
                    self.set_color(x_parent, Color::Red);
                    self.rotate_left(x_parent);
                    w = self.node(x_parent).right;
                }

                if self.color_of(self.left_of(w)) == Color::Black
                    && self.color_of(self.right_of(w)) == Color::Black
                {
                    self.set_color(w, Color::Red);
                    x = x_parent;
                    x_parent = self.parent_of(x);
                } else {
                    if self.color_of(self.right_of(w)) == Color::Black {
                        self.set_color(self.left_of(w), Color::Black);
                        self.set_color(w, Color::Red);
                        self.rotate_right(w);
                        w = self.node(x_parent).right;
                    }
                    self.set_color(w, self.color_of(x_parent));
                    self.set_color(x_parent, Color::Black);
                    self.set_color(self.right_of(w), Color::Black);
                    self.rotate_left(x_parent);
                    x = self.root;
                    x_parent = NIL;
                }
            } else {
                let mut w = self.node(x_parent).left;

                if self.color_of(w) == Color::Red {
                    self.set_color(w, Color::Black);
                    self.set_color(x_parent, Color::Red);
                    self.rotate_right(x_parent);
                    w = self.node(x_parent).left;
                }

                if self.color_of(self.right_of(w)) == Color::Black
                    && self.color_of(self.left_of(w)) == Color::Black
                {
                    self.set_color(w, Color::Red);
                    x = x_parent;
                    x_parent = self.parent_of(x);
                } else {
                    if self.color_of(self.left_of(w)) == Color::Black {
                        self.set_color(self.right_of(w), Color::Black);
                        self.set_color(w, Color::Red);
                        self.rotate_left(w);
                        w = self.node(x_parent).left;
                    }
                    self.set_color(w, self.color_of(x_parent));
                    self.set_color(x_parent, Color::Black);
                    self.set_color(self.left_of(w), Color::Black);
                    self.rotate_right(x_parent);
                    x = self.root;
                    x_parent = NIL;
                }
            }
        }
        if x != NIL {
            self.set_color(x, Color::Black);
        }
    }

    fn rotate_left(&mut self, x: u32) {
        let y = self.node(x).right;
        let yl = self.node(y).left;
        let xp = self.node(x).parent;

        self.node_mut(x).right = yl;
        if yl != NIL {
            self.node_mut(yl).parent = x;
        }

        self.node_mut(y).parent = xp;
        if xp == NIL {
            self.root = y;
        } else if x == self.node(xp).left {
            self.node_mut(xp).left = y;
        } else {
            self.node_mut(xp).right = y;
        }

        self.node_mut(y).left = x;
        self.node_mut(x).parent = y;

        self.recompute_min_vruntime(x);
        self.recompute_min_vruntime(y);
    }

    fn rotate_right(&mut self, y: u32) {
        let x = self.node(y).left;
        let xr = self.node(x).right;
        let yp = self.node(y).parent;

        self.node_mut(y).left = xr;
        if xr != NIL {
            self.node_mut(xr).parent = y;
        }

        self.node_mut(x).parent = yp;
        if yp == NIL {
            self.root = x;
        } else if y == self.node(yp).left {
            self.node_mut(yp).left = x;
        } else {
            self.node_mut(yp).right = x;
        }

        self.node_mut(x).right = y;
        self.node_mut(y).parent = x;

        self.recompute_min_vruntime(y);
        self.recompute_min_vruntime(x);
    }

    /// Replaces the subtree rooted at `u` with the subtree rooted at `v`.
    fn transplant(&mut self, u: u32, v: u32) {
        let up = self.node(u).parent;
        if up == NIL {
            self.root = v;
        } else if u == self.node(up).left {
            self.node_mut(up).left = v;
        } else {
            self.node_mut(up).right = v;
        }
        if v != NIL {
            self.node_mut(v).parent = up;
        }
    }

    // -- Arena and node accessors --

    fn tree_minimum(&self, mut idx: u32) -> u32 {
        while self.node(idx).left != NIL {
            idx = self.node(idx).left;
        }
        idx
    }

    fn alloc_node(&mut self, data: TaskData<T>, color: Color) -> u32 {
        let min_vruntime = data.vruntime;
        let node = Node {
            data,
            color,
            parent: NIL,
            left: NIL,
            right: NIL,
            min_vruntime,
        };
        if let Some(idx) = self.free_list.pop() {
            self.nodes[idx as usize] = Some(node);
            idx
        } else {
            let idx = self.nodes.len() as u32;
            self.nodes.push(Some(node));
            idx
        }
    }

    fn free_node(&mut self, idx: u32) -> TaskData<T> {
        let node = self.nodes[idx as usize]
            .take()
            .expect("freeing absent node");
        self.free_list.push(idx);
        node.data
    }

    fn node(&self, idx: u32) -> &Node<T> {
        self.nodes[idx as usize]
            .as_ref()
            .expect("accessing absent node")
    }

    fn node_mut(&mut self, idx: u32) -> &mut Node<T> {
        self.nodes[idx as usize]
            .as_mut()
            .expect("accessing absent node")
    }

    fn color_of(&self, idx: u32) -> Color {
        if idx == NIL {
            Color::Black
        } else {
            self.node(idx).color
        }
    }

    fn set_color(&mut self, idx: u32, color: Color) {
        if idx != NIL {
            self.node_mut(idx).color = color;
        }
    }

    fn parent_of(&self, idx: u32) -> u32 {
        if idx == NIL {
            NIL
        } else {
            self.node(idx).parent
        }
    }

    fn left_of(&self, idx: u32) -> u32 {
        if idx == NIL { NIL } else { self.node(idx).left }
    }

    fn right_of(&self, idx: u32) -> u32 {
        if idx == NIL {
            NIL
        } else {
            self.node(idx).right
        }
    }

    fn recompute_min_vruntime(&mut self, idx: u32) {
        if idx == NIL {
            return;
        }
        let left = self.node(idx).left;
        let right = self.node(idx).right;
        let mut min_v = self.node(idx).data.vruntime;
        if left != NIL {
            min_v = min_v.min(self.node(left).min_vruntime);
        }
        if right != NIL {
            min_v = min_v.min(self.node(right).min_vruntime);
        }
        self.node_mut(idx).min_vruntime = min_v;
    }

    /// Walks from `idx` to the root, recomputing `min_vruntime` at every node.
    fn update_min_vruntime_to_root(&mut self, mut idx: u32) {
        while idx != NIL {
            self.recompute_min_vruntime(idx);
            idx = self.node(idx).parent;
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

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

    /// Pops the earliest-deadline task, ignoring eligibility.
    ///
    /// Equivalent to `pop` with parameters that make every task eligible:
    /// `(vruntime - 0) * 0 = 0 <= 0` is always true.
    fn pop_earliest(queue: &mut EligibilityQueue<u64>) -> Option<TaskData<u64>> {
        queue.pop(0, 0, 0)
    }

    /// Recursively verifies red-black tree invariants and returns the black
    /// height of the subtree rooted at `idx`.
    fn verify_subtree<T>(queue: &EligibilityQueue<T>, idx: u32) -> u32 {
        if idx == NIL {
            return 1;
        }

        let node = queue.node(idx);
        let left = node.left;
        let right = node.right;

        // Check parent pointers.
        if left != NIL {
            assert_eq!(queue.node(left).parent, idx, "Left child's parent mismatch");
        }
        if right != NIL {
            assert_eq!(
                queue.node(right).parent,
                idx,
                "Right child's parent mismatch"
            );
        }

        // Red nodes must have black children.
        if node.color == Color::Red {
            assert_eq!(
                queue.color_of(left),
                Color::Black,
                "Red node has red left child"
            );
            assert_eq!(
                queue.color_of(right),
                Color::Black,
                "Red node has red right child"
            );
        }

        // Check BST ordering.
        if left != NIL {
            assert!(
                queue.node(left).data < node.data,
                "BST ordering violated on left"
            );
        }
        if right != NIL {
            assert!(
                node.data < queue.node(right).data,
                "BST ordering violated on right"
            );
        }

        // Check min_vruntime augmentation.
        let mut expected_min = node.data.vruntime;
        if left != NIL {
            expected_min = expected_min.min(queue.node(left).min_vruntime);
        }
        if right != NIL {
            expected_min = expected_min.min(queue.node(right).min_vruntime);
        }
        assert_eq!(
            node.min_vruntime, expected_min,
            "min_vruntime calculation incorrect"
        );

        // Check black-height balance.
        let left_bh = verify_subtree(queue, left);
        let right_bh = verify_subtree(queue, right);
        assert_eq!(
            left_bh, right_bh,
            "Black height mismatch: left={}, right={}",
            left_bh, right_bh
        );

        left_bh + if node.color == Color::Black { 1 } else { 0 }
    }

    fn verify_tree_invariants<T>(queue: &EligibilityQueue<T>) {
        if queue.root != NIL {
            assert_eq!(
                queue.node(queue.root).color,
                Color::Black,
                "Root must be black"
            );
            assert_eq!(
                queue.node(queue.root).parent,
                NIL,
                "Root's parent must be NIL"
            );
        }
        verify_subtree(queue, queue.root);
    }

    #[ktest]
    fn empty_tree() {
        let queue: EligibilityQueue<&str> = EligibilityQueue::new();
        assert!(queue.len() == 0);
        assert!(queue.min_vruntime().is_none());

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn single_node() {
        let mut queue = EligibilityQueue::new();
        let task = create_task(1, 100, 10, 50);

        queue.push(task);
        assert!(queue.len() != 0);
        assert_eq!(queue.min_vruntime(), Some(50));

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn multiple_nodes_ordered_insertion() {
        let mut queue = EligibilityQueue::new();

        // Insert in increasing order of vdeadline
        for i in 1..=10i64 {
            queue.push(create_task(i as u64, i * 10, 10, i * 5));
        }

        assert!(queue.len() != 0);
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn multiple_nodes_reverse_ordered_insertion() {
        let mut queue = EligibilityQueue::new();

        // Insert in decreasing order of vdeadline
        for i in (1..=10i64).rev() {
            queue.push(create_task(i as u64, i * 10, 10, i * 5));
        }

        assert!(queue.len() != 0);
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn multiple_nodes_random_insertion() {
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

        assert!(queue.len() != 0);
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn pop_earliest_basic() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(2, 20, 10, 10));
        queue.push(create_task(1, 10, 10, 5));
        queue.push(create_task(3, 30, 10, 15));

        let min_task = pop_earliest(&mut queue);
        assert_eq!(min_task.unwrap().id, 1);

        verify_tree_invariants(&queue);

        let second_min = pop_earliest(&mut queue);
        assert_eq!(second_min.unwrap().id, 2);

        verify_tree_invariants(&queue);

        let third_min = pop_earliest(&mut queue);
        assert_eq!(third_min.unwrap().id, 3);

        assert!(queue.len() == 0);
    }

    #[ktest]
    fn pop_eligible_all_eligible() {
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
    fn pop_eligible_none_eligible() {
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
    fn pop_eligible_some_eligible() {
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

        assert!(queue.len() == 0);
    }

    #[ktest]
    fn pop_complex_eligibility() {
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
    fn pop_eligible_with_different_weights() {
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
    fn pop_changing_global_min_vruntime() {
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
    fn rotation_cases() {
        // Test insertion patterns that trigger rotations.
        let mut queue = EligibilityQueue::new();
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(2, 20, 10, 20));
        queue.push(create_task(3, 30, 10, 30));

        verify_tree_invariants(&queue);

        let mut queue = EligibilityQueue::new();
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(2, 20, 10, 20));
        queue.push(create_task(1, 10, 10, 10));

        verify_tree_invariants(&queue);

        let mut queue = EligibilityQueue::new();
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(2, 20, 10, 20));

        verify_tree_invariants(&queue);

        let mut queue = EligibilityQueue::new();
        queue.push(create_task(1, 10, 10, 10));
        queue.push(create_task(3, 30, 10, 30));
        queue.push(create_task(2, 20, 10, 20));

        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn min_vruntime_calculation() {
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
    fn task_ordering() {
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
    fn stress_large_tree() {
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
            let task = pop_earliest(&mut queue).expect("Should have tasks remaining");
            assert!(task.vdeadline >= last_vdeadline);
            last_vdeadline = task.vdeadline;

            verify_tree_invariants(&queue);
        }

        assert!(queue.len() == 0);
    }

    #[ktest]
    fn pop_with_global_min_vruntime_updates() {
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
    fn edge_case_vruntime_values() {
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
    fn mixed_operations() {
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
            let task = pop_earliest(&mut queue).expect("Should have task");
            assert_eq!(task.id, expected_id);
            if queue.len() != 0 {
                verify_tree_invariants(&queue);
            }
        }

        assert!(queue.len() == 0);
    }

    #[ktest]
    fn min_vruntime_after_pop() {
        let mut queue = EligibilityQueue::new();

        // Task 3 has the minimum vruntime (10).
        queue.push(create_task(1, 10, 10, 50));
        queue.push(create_task(2, 20, 10, 10));
        queue.push(create_task(3, 30, 10, 30));

        assert_eq!(queue.min_vruntime(), Some(10));

        // Pop task 2 (vr=10) by making only it eligible.
        // (vr - 0) * 30 ≤ offset: task 2 → 300, task 1 → 1500, task 3 → 900.
        let task = queue.pop(0, 30, 300);
        assert_eq!(task.unwrap().id, 2);

        // After removing task 2 (vr=10), min should be 30 (task 3).
        assert_eq!(queue.min_vruntime(), Some(30));
        verify_tree_invariants(&queue);

        // Pop task 1 (the leftmost by vdeadline).
        let task = pop_earliest(&mut queue).unwrap();
        assert_eq!(task.id, 1);
        assert_eq!(queue.min_vruntime(), Some(30));
        verify_tree_invariants(&queue);

        // Pop the last task.
        let task = pop_earliest(&mut queue).unwrap();
        assert_eq!(task.id, 3);
        assert_eq!(queue.min_vruntime(), None);
    }

    #[ktest]
    fn min_vruntime_against_method() {
        let mut queue: EligibilityQueue<u64> = EligibilityQueue::new();

        // Empty queue: returns the argument unchanged.
        assert_eq!(queue.min_vruntime_against(100), 100);
        assert_eq!(queue.min_vruntime_against(-50), -50);

        // Non-empty: returns the min of argument and queue's min_vruntime.
        queue.push(create_task(1, 10, 10, 40));
        assert_eq!(queue.min_vruntime_against(50), 40);
        assert_eq!(queue.min_vruntime_against(30), 30);
        assert_eq!(queue.min_vruntime_against(40), 40);
    }

    #[ktest]
    fn delete_two_children_non_direct_successor() {
        let mut queue = EligibilityQueue::new();

        // Build tree:
        //       20(B)
        //      / \
        //    10(B) 30(B)
        //          / \
        //        25(R) 35(R)
        //
        // Deleting node 20 triggers the two-children case where the in-order
        // successor (25) is not 20's direct right child (30).
        queue.push(create_task(20, 20, 10, 5));
        queue.push(create_task(10, 10, 10, 1000));
        queue.push(create_task(30, 30, 10, 1000));
        queue.push(create_task(25, 25, 10, 1000));
        queue.push(create_task(35, 35, 10, 1000));

        verify_tree_invariants(&queue);

        // Pop node 20 via eligibility: only node 20 (vr=5) is eligible.
        // (vr - 0) * 50 ≤ 250: node 20 → 250 ✓, all others → ≥50000 ✗
        let task = queue.pop(0, 50, 250);
        assert_eq!(task.unwrap().id, 20);
        verify_tree_invariants(&queue);

        // Remaining nodes in vdeadline order.
        for expected_id in [10, 25, 30, 35] {
            assert_eq!(pop_earliest(&mut queue).unwrap().id, expected_id);
        }
        assert!(queue.len() == 0);
    }

    #[ktest]
    fn negative_vruntimes() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, -100));
        queue.push(create_task(2, 20, 10, -50));
        queue.push(create_task(3, 30, 10, 25));

        assert_eq!(queue.min_vruntime(), Some(-100));
        verify_tree_invariants(&queue);

        // global_min = -100. Eligibility: (vr - (-100)) * 30 ≤ 500
        // Task 1: 0 ≤ 500 ✓, Task 2: 1500 > 500 ✗, Task 3: 3750 > 500 ✗
        let task = queue.pop(-100, 30, 500);
        assert_eq!(task.unwrap().id, 1);

        assert_eq!(queue.min_vruntime(), Some(-50));
        verify_tree_invariants(&queue);

        // Both remaining eligible with generous offset.
        let task = queue.pop(-50, 20, 100_000);
        assert_eq!(task.unwrap().id, 2);
        assert_eq!(queue.min_vruntime(), Some(25));
    }

    #[ktest]
    fn free_list_reuse() {
        let mut queue = EligibilityQueue::new();

        for i in 0..10u64 {
            queue.push(create_task(i, i as i64, 10, i as i64));
        }
        let arena_size = queue.nodes.len();
        assert_eq!(arena_size, 10);

        // Pop all.
        while pop_earliest(&mut queue).is_some() {}
        assert!(queue.len() == 0);

        // Arena hasn't shrunk, but free list has all slots.
        assert_eq!(queue.nodes.len(), arena_size);
        assert_eq!(queue.free_list.len(), arena_size);

        // Push again — should reuse freed slots, not grow the arena.
        for i in 10..20u64 {
            queue.push(create_task(i, i as i64, 10, i as i64));
        }
        assert_eq!(queue.nodes.len(), arena_size);
        assert!(queue.free_list.is_empty());
        verify_tree_invariants(&queue);
    }

    #[ktest]
    fn upsert_replaces_data() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 100));
        queue.push(create_task(2, 20, 10, 200));
        queue.push(create_task(3, 30, 10, 300));

        assert_eq!(queue.min_vruntime(), Some(100));

        // Push with the same (vdeadline=20, id=2) but different vruntime.
        queue.push(create_task(2, 20, 10, 50));

        assert_eq!(queue.min_vruntime(), Some(50));
        verify_tree_invariants(&queue);

        // Pop all — the replacement should be present.
        let t1 = pop_earliest(&mut queue).unwrap();
        assert_eq!((t1.id, t1.vruntime), (1, 100));
        let t2 = pop_earliest(&mut queue).unwrap();
        assert_eq!((t2.id, t2.vruntime), (2, 50));
        let t3 = pop_earliest(&mut queue).unwrap();
        assert_eq!((t3.id, t3.vruntime), (3, 300));
    }

    #[ktest]
    fn eligibility_tie_breaking_by_id() {
        let mut queue = EligibilityQueue::new();

        // Same vdeadline, different IDs. Ord breaks ties by id.
        queue.push(create_task(3, 100, 10, 20));
        queue.push(create_task(1, 100, 10, 10));
        queue.push(create_task(2, 100, 10, 15));

        verify_tree_invariants(&queue);

        // All eligible — should return smallest id first.
        assert_eq!(queue.pop(0, 30, 100_000).unwrap().id, 1);
        assert_eq!(queue.pop(0, 20, 100_000).unwrap().id, 2);
        assert_eq!(queue.pop(0, 10, 100_000).unwrap().id, 3);
    }

    #[ktest]
    fn single_element_eligible_pop() {
        let mut queue = EligibilityQueue::new();

        queue.push(create_task(1, 10, 10, 50));

        let task = queue.pop(0, 10, 1000);
        assert_eq!(task.unwrap().id, 1);

        assert!(queue.len() == 0);
        assert_eq!(queue.min_vruntime(), None);
    }

    #[ktest]
    fn delete_internal_nodes() {
        let mut queue = EligibilityQueue::new();

        // 15 nodes with distinct vruntimes scattered across the tree.
        // vruntime ordering: 14(5), 12(10), 10(25), 8(50), 5(100),
        //                    4(200), 3(300), 2(400), 1(500), 6(600),
        //                    7(700), 9(800), 11(900), 13(950), 15(1000)
        let tasks = [
            create_task(1, 10, 10, 500),
            create_task(2, 20, 10, 400),
            create_task(3, 30, 10, 300),
            create_task(4, 40, 10, 200),
            create_task(5, 50, 10, 100),
            create_task(6, 60, 10, 600),
            create_task(7, 70, 10, 700),
            create_task(8, 80, 10, 50),
            create_task(9, 90, 10, 800),
            create_task(10, 100, 10, 25),
            create_task(11, 110, 10, 900),
            create_task(12, 120, 10, 10),
            create_task(13, 130, 10, 950),
            create_task(14, 140, 10, 5),
            create_task(15, 150, 10, 1000),
        ];
        for task in tasks {
            queue.push(task);
        }
        verify_tree_invariants(&queue);

        // Pop nodes scattered through the tree by targeting specific
        // vruntimes via eligibility: (vr - 0) * total_weight ≤ offset.

        // Pop id=14 (vr=5): total=150, need offset ≥ 750; next vr=10 → 1500
        assert_eq!(queue.pop(0, 150, 750).unwrap().id, 14);
        verify_tree_invariants(&queue);

        // Pop id=12 (vr=10): total=140, offset=1400; next vr=25 → 3500
        assert_eq!(queue.pop(0, 140, 1400).unwrap().id, 12);
        verify_tree_invariants(&queue);

        // Pop id=10 (vr=25): total=130, offset=3250; next vr=50 → 6500
        assert_eq!(queue.pop(0, 130, 3250).unwrap().id, 10);
        verify_tree_invariants(&queue);

        // Pop id=8 (vr=50): total=120, offset=6000; next vr=100 → 12000
        assert_eq!(queue.pop(0, 120, 6000).unwrap().id, 8);
        verify_tree_invariants(&queue);

        // Pop id=5 (vr=100): total=110, offset=11000; next vr=200 → 22000
        assert_eq!(queue.pop(0, 110, 11000).unwrap().id, 5);
        verify_tree_invariants(&queue);

        // Pop the remaining 10 by vdeadline order.
        for expected_id in [1, 2, 3, 4, 6, 7, 9, 11, 13, 15] {
            assert_eq!(pop_earliest(&mut queue).unwrap().id, expected_id);
            if queue.len() != 0 {
                verify_tree_invariants(&queue);
            }
        }
        assert!(queue.len() == 0);
    }

    #[ktest]
    fn min_vruntime_maintained_through_operations() {
        let mut queue = EligibilityQueue::new();

        // Insert with vruntimes that don't follow vdeadline order.
        let vruntimes = [50i64, 30, 70, 10, 90, 20, 80];
        for (i, &vr) in vruntimes.iter().enumerate() {
            queue.push(create_task(i as u64, (i as i64 + 1) * 10, 10, vr));
            let expected_min = vruntimes[..=i].iter().copied().min().unwrap();
            assert_eq!(queue.min_vruntime(), Some(expected_min));
            verify_tree_invariants(&queue);
        }

        // Pop by vdeadline and track the evolving minimum vruntime.
        // Remaining vruntimes after each pop:
        //   pop id=0 (vr=50): {30, 70, 10, 90, 20, 80} → min 10
        //   pop id=1 (vr=30): {70, 10, 90, 20, 80}     → min 10
        //   pop id=2 (vr=70): {10, 90, 20, 80}          → min 10
        //   pop id=3 (vr=10): {90, 20, 80}              → min 20
        //   pop id=4 (vr=90): {20, 80}                  → min 20
        //   pop id=5 (vr=20): {80}                      → min 80
        let expected_mins = [10, 10, 10, 20, 20, 80];
        for &expected_min in &expected_mins {
            pop_earliest(&mut queue);
            assert_eq!(queue.min_vruntime(), Some(expected_min));
            verify_tree_invariants(&queue);
        }

        pop_earliest(&mut queue);
        assert_eq!(queue.min_vruntime(), None);
    }

    #[ktest]
    fn stress_pop_with_mixed_eligibility() {
        let mut queue = EligibilityQueue::new();
        const COUNT: usize = 200;

        for i in 0..COUNT {
            // Scatter vruntimes: every 3rd task has a low vruntime (eligible).
            let vruntime = if i % 3 == 0 {
                (i as i64) * 2
            } else {
                1000 + i as i64
            };
            queue.push(create_task(i as u64, (i * 10) as i64, 10, vruntime));
        }
        verify_tree_invariants(&queue);

        let mut popped = 0;
        while queue.len() != 0 {
            let remaining = COUNT - popped;
            let total_weight = (remaining * 10) as i64;
            let _ = queue.pop(0, total_weight, 50_000);
            popped += 1;
            verify_tree_invariants(&queue);
        }
        assert_eq!(popped, COUNT);
    }

    #[ktest]
    fn push_pop_interleave() {
        let mut queue = EligibilityQueue::new();

        // Rapidly alternate push/pop at small tree sizes.
        for round in 0..50u64 {
            queue.push(create_task(
                round * 2,
                (round * 2) as i64 * 10,
                10,
                (round * 2) as i64,
            ));
            queue.push(create_task(
                round * 2 + 1,
                (round * 2 + 1) as i64 * 10,
                10,
                (round * 2 + 1) as i64,
            ));
            verify_tree_invariants(&queue);

            pop_earliest(&mut queue);
            verify_tree_invariants(&queue);
        }

        // Drain remaining.
        while pop_earliest(&mut queue).is_some() {
            verify_tree_invariants(&queue);
        }
        assert!(queue.len() == 0);
    }
}
