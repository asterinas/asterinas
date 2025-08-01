// SPDX-License-Identifier: MPL-2.0

//! A linked list implementation using RCU synchronization.
//!
//! This module provides an RCU-based doubly-linked list ([`RcuList`]) designed for high-performance
//! concurrent access scenarios where reads vastly outnumber writes.

use ostd::{
    sync::{non_null::ArcRef, RcuOption},
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
};

use crate::prelude::*;

/// A linked list implementation using RCU (Read-Copy-Update) synchronization.
pub struct RcuList<T: 'static + Send + Sync> {
    head: RcuOption<Arc<T>>,
    obtain_link: fn(&T) -> &RcuListLink<T>,
}

/// The link fields for a node in the RCU linked list.
pub struct RcuListLink<T: 'static + Send + Sync> {
    next: RcuOption<Arc<T>>,
    prev: RcuOption<Arc<T>>,
}

impl<T: 'static + Send + Sync> RcuList<T> {
    /// Creates a new empty RCU linked list
    ///
    /// # Example
    /// ```
    /// struct Node {
    ///     value: usize,
    ///     link: RcuListLink<Self>
    /// }
    ///
    /// let node_list = RcuLinkedList::new(|node| &node.link);
    /// ```
    pub fn new(obtain_link: fn(&T) -> &RcuListLink<T>) -> Self {
        RcuList {
            head: RcuOption::new_none(),
            obtain_link,
        }
    }

    /// Checks if the list is empty
    pub fn is_empty(&self, guard: &dyn AsAtomicModeGuard) -> bool {
        self.head.read_with(guard).is_none()
    }

    /// Returns a reference to the front node in the list.
    pub fn front<'a>(&'a self, guard: &'a dyn AsAtomicModeGuard) -> Option<ArcRef<'a, T>> {
        self.head.read_with(guard)
    }

    /// Pushes a new node to the front of the list
    pub fn push_front(&self, new_head: Arc<T>, guard: &dyn AsAtomicModeGuard) {
        let _guard = guard.as_atomic_mode_guard();

        let head = self.head.read_with(guard);
        let new_link = (self.obtain_link)(&new_head);

        new_link.prev.update(None);
        if let Some(head) = head {
            let head_link = (self.obtain_link)(&head);

            new_link.next.update(Some(head.clone()));
            head_link.prev.update(Some(new_head.clone()));
        } else {
            new_link.next.update(None);
        }

        self.head.update(Some(new_head));
    }

    /// Pushes a new node to the front of the list
    pub fn pop_front(&self, guard: &dyn AsAtomicModeGuard) -> Option<Arc<T>> {
        let _guard = guard.as_atomic_mode_guard();

        let head = self.head.read_with(guard)?.clone();

        let head_link = (self.obtain_link)(&head);
        let new_head = head_link.next.read_with(guard);
        if let Some(new_head) = &new_head {
            let new_head_link = (self.obtain_link)(new_head);
            new_head_link.prev.update(None);
        }

        self.head.update(new_head.as_deref().cloned());

        Some(head)
    }

    /// Removes a node from the list
    pub fn remove(&self, node: &T, guard: &dyn AsAtomicModeGuard) {
        let _guard = guard.as_atomic_mode_guard();

        let links = (self.obtain_link)(node);

        let prev = links.prev.read_with(guard);
        let next = links.next.read_with(guard);

        if let Some(prev_node) = &prev {
            let prev_links = (self.obtain_link)(prev_node);
            prev_links.next.update(next.as_deref().cloned());
        } else {
            self.head.update(next.as_deref().cloned());
        }

        if let Some(next_node) = next {
            let next_links = (self.obtain_link)(&next_node);
            next_links.prev.update(prev.as_deref().cloned());
        }

        links.next.update(None);
        links.prev.update(None);
    }

    /// Returns a iterator over the list.
    pub fn iter<'a>(&'a self, guard: &'a dyn AsAtomicModeGuard) -> RcuListIter<'a, T> {
        RcuListIter {
            list: self,
            guard,
            current: self.head.read_with(guard).as_deref().cloned(),
        }
    }
}

impl<T: 'static + Send + Sync> Default for RcuListLink<T> {
    fn default() -> Self {
        Self {
            next: RcuOption::new_none(),
            prev: RcuOption::new_none(),
        }
    }
}

/// An iterator over the nodes of an `RcuList`.
pub struct RcuListIter<'a, T: 'static + Send + Sync> {
    list: &'a RcuList<T>,
    guard: &'a dyn AsAtomicModeGuard,
    current: Option<Arc<T>>,
}

impl<T: 'static + Send + Sync> Iterator for RcuListIter<'_, T> {
    type Item = Arc<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let current_node = self.current.take()?;

        let link = (self.list.obtain_link)(&current_node);
        self.current = link.next.read_with(self.guard).as_deref().cloned();

        Some(current_node)
    }
}

impl<T: 'static + Send + Sync> Drop for RcuList<T> {
    fn drop(&mut self) {
        let current_rcu = core::mem::replace(&mut self.head, RcuOption::new_none());

        let guard = disable_preempt();

        let mut head_node = current_rcu.read_with(&guard).as_deref().cloned();
        while let Some(node) = head_node {
            let current_link = (self.obtain_link)(&node);
            let next_node = current_link.next.read_with(&guard).as_deref().cloned();

            current_link.next.update(None);
            current_link.prev.update(None);

            head_node = next_node;
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::{prelude::*, task::disable_preempt};

    use super::*;

    #[derive(Default)]
    struct TestNode {
        id: u32,
        link: RcuListLink<Self>,
    }

    #[ktest]
    fn test_empty_list() {
        let list = RcuList::<TestNode>::new(|n| &n.link);
        let guard = disable_preempt();

        assert!(list.is_empty(&guard));
        assert!(list.front(&guard).is_none());
        assert!(list.pop_front(&guard).is_none());
    }

    #[ktest]
    fn test_push_pop_front() {
        let list = RcuList::<TestNode>::new(|n| &n.link);
        let guard = disable_preempt();

        let node1 = Arc::new(TestNode {
            id: 1,
            ..Default::default()
        });
        let node2 = Arc::new(TestNode {
            id: 2,
            ..Default::default()
        });

        list.push_front(node1.clone(), &guard);
        assert_eq!(list.front(&guard).unwrap().id, 1);

        list.push_front(node2.clone(), &guard);
        assert_eq!(list.front(&guard).unwrap().id, 2);

        assert_eq!(list.pop_front(&guard).unwrap().id, 2);
        assert_eq!(list.pop_front(&guard).unwrap().id, 1);
        assert!(list.is_empty(&guard));
    }

    #[ktest]
    fn test_remove_middle() {
        let list = RcuList::<TestNode>::new(|n| &n.link);
        let guard = disable_preempt();

        let nodes = vec![
            Arc::new(TestNode {
                id: 1,
                ..Default::default()
            }),
            Arc::new(TestNode {
                id: 2,
                ..Default::default()
            }),
            Arc::new(TestNode {
                id: 3,
                ..Default::default()
            }),
        ];

        for node in &nodes {
            list.push_front(node.clone(), &guard);
        }

        // Remove middle node (id=2)
        list.remove(&nodes[1], &guard);

        let mut iter = list.iter(&guard);
        assert_eq!(iter.next().unwrap().id, 3);
        assert_eq!(iter.next().unwrap().id, 1);
        assert!(iter.next().is_none());
    }

    #[ktest]
    fn test_drop_cleanup() {
        let list = RcuList::<TestNode>::new(|n| &n.link);
        let guard = disable_preempt();

        let node = Arc::new(TestNode {
            id: 1,
            ..Default::default()
        });
        list.push_front(node.clone(), &guard);

        drop(list);

        // Verify node links were cleared
        assert!(node.link.next.read_with(&guard).is_none());
        assert!(node.link.prev.read_with(&guard).is_none());
    }
}
