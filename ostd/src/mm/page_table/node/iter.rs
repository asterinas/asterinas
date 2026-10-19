// SPDX-License-Identifier: MPL-2.0

//! Owning iteration over a node's backing frames, with optional RCU reclamation.

use core::mem::ManuallyDrop;

use super::{PageTableConfig, PageTableNode, PageTablePageMeta};
use crate::{
    mm::frame::{Frame, meta::AnyFrameMeta, segment},
    sync::RcuDrop,
};

impl<C: PageTableConfig> IntoIterator for PageTableNode<C> {
    type Item = Frame<PageTablePageMeta<C>>;
    type IntoIter = segment::IntoIter<PageTablePageMeta<C>>;

    fn into_iter(self) -> Self::IntoIter {
        self.frames.into_iter()
    }
}

impl<C: PageTableConfig> IntoIterator for RcuDrop<PageTableNode<C>> {
    type Item = RcuDrop<Frame<dyn AnyFrameMeta>>;
    type IntoIter = RcuIntoIter<C>;

    fn into_iter(self) -> Self::IntoIter {
        // SAFETY: Both yielded frames and unconsumed frames remain RCU-dropped.
        let (node, panic_guard) = unsafe { RcuDrop::into_inner(self) };
        let iter = RcuIntoIter {
            inner: ManuallyDrop::new(node.into_iter()),
        };
        panic_guard.forget();
        iter
    }
}

/// An owning iterator yielding RCU-dropped backing frames in physical order.
///
/// Dropping it defers reclamation of any unconsumed frames until an RCU grace
/// period has elapsed. Callers remain responsible for TLB coherence before
/// dropping the yielded frames or the iterator.
pub(crate) struct RcuIntoIter<C: PageTableConfig> {
    inner: ManuallyDrop<segment::IntoIter<PageTablePageMeta<C>>>,
}

impl<C: PageTableConfig> Iterator for RcuIntoIter<C> {
    type Item = RcuDrop<Frame<dyn AnyFrameMeta>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|frame| RcuDrop::new(frame.into()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<C: PageTableConfig> ExactSizeIterator for RcuIntoIter<C> {}

impl<C: PageTableConfig> Drop for RcuIntoIter<C> {
    fn drop(&mut self) {
        // SAFETY: `inner` is taken exactly once and never accessed again.
        let remaining = unsafe { ManuallyDrop::take(&mut self.inner) };
        if remaining.len() != 0 {
            drop(RcuDrop::new(remaining));
        }
    }
}
