// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use super::*;
use crate::{
    cpu,
    mm::page_table::test::variable_vpn_width::WideConfig,
    prelude::ktest,
    sync::RcuDrop,
    task::{Task, TaskOptions, disable_preempt},
};

#[ktest]
fn consuming_nodes_yields_frames_in_order_and_releases_the_remainder() {
    for level in [1, 3] {
        let node = PageTableNode::<WideConfig>::alloc(level);
        let count = page_table_node_size::<WideConfig>(level) / PAGE_SIZE;
        let observers: Vec<_> = (0..count)
            .map(|page| node.borrow().frame(page).clone())
            .collect();
        let mut iter = node.into_iter();
        assert_eq!(iter.len(), count);
        let first = iter.next().unwrap();
        assert_eq!(first.paddr(), observers[0].paddr());
        assert_eq!(iter.len(), count - 1);
        drop(first);
        assert_eq!(observers[0].reference_count(), 1);
        for frame in &observers[1..] {
            assert_eq!(frame.reference_count(), 2);
        }
        drop(iter);
        for frame in observers {
            assert_eq!(frame.reference_count(), 1);
        }
    }
}

#[ktest]
fn consuming_rcu_nodes_defers_yielded_and_unconsumed_frames() {
    // Exercise dropping before iteration, partway through it, and after exhaustion.
    for consumed in [0, 1, 4] {
        let atomic_guard = disable_preempt();
        let node = PageTableNode::<WideConfig>::alloc(3);
        let observers: Vec<_> = (0..4)
            .map(|page| node.borrow().frame(page).clone())
            .collect();
        let mut iter = RcuDrop::new(node).into_iter();
        assert_eq!(iter.len(), 4);
        for (page, observer) in observers.iter().enumerate().take(consumed) {
            let frame = iter.next().unwrap();
            assert_eq!(frame.paddr(), observer.paddr());
            assert_eq!(iter.len(), 3 - page);
            drop(frame);
        }
        if consumed == 4 {
            assert!(iter.next().is_none());
            assert!(iter.next().is_none());
        }
        drop(iter);
        for frame in &observers {
            assert_eq!(frame.reference_count(), 2);
        }
        drop(atomic_guard);

        // The default test scheduler runs tasks only on the BSP. On UP, a
        // switch to a helper and back completes both queued grace periods.
        // On SMP, the checks above still verify that reclamation is deferred.
        if cpu::num_cpus() == 1 {
            TaskOptions::new(|| {}).data(()).spawn().unwrap();
            Task::yield_now();
            for frame in &observers {
                assert_eq!(frame.reference_count(), 1);
            }
        }
    }
}

#[ktest]
fn guards_round_and_release_only_their_pages() {
    let node = PageTableNode::<WideConfig>::alloc(3);
    let atomic_guard = disable_preempt();
    let middle = node.borrow().lock(&atomic_guard, 513..1535);
    assert_eq!(middle.locked_entries(), 512..1536);
    let first = node.borrow().lock(&atomic_guard, 0..1);
    let last = node.borrow().lock(&atomic_guard, 2047..2048);

    drop(middle);
    let second = node.borrow().lock(&atomic_guard, 512..513);
    let third = node.borrow().lock(&atomic_guard, 1024..1025);

    // All four locks must still be owned by separate guards. In particular,
    // dropping one guard must not unlock a neighboring guard.
    for page in 0..4 {
        assert_eq!(
            node.borrow()
                .frame(page)
                .meta()
                .lock
                .load(Ordering::Relaxed),
            1
        );
    }
    drop((first, second, third, last));
    let whole = node.borrow().lock(&atomic_guard, 0..2048);
    assert_eq!(whole.locked_entries(), 0..2048);
}

#[ktest]
#[should_panic(expected = "self.locked_entries().contains(&idx)")]
fn guards_reject_entries_outside_locked_pages() {
    let node = PageTableNode::<WideConfig>::alloc(3);
    let atomic_guard = disable_preempt();
    let mut guard = node.borrow().lock(&atomic_guard, 512..513);
    guard.entry(511);
}

#[ktest]
#[should_panic]
fn partial_guards_cannot_mark_a_node_stray() {
    let node = PageTableNode::<WideConfig>::alloc(3);
    let atomic_guard = disable_preempt();
    node.borrow().lock(&atomic_guard, 512..513).mark_stray();
}
