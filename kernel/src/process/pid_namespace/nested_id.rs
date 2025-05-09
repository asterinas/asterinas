// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use super::PidNamespace;
use crate::prelude::*;

#[derive(Clone)]
pub struct NestedId(pub(super) Arc<VecDeque<UniqueId>>);

impl PartialEq for NestedId {
    fn eq(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }

        for (id1, id2) in self.0.iter().zip(other.0.iter()) {
            if id1.id != id2.id {
                return false;
            }
        }

        true
    }
}

impl Eq for NestedId {}

impl PartialOrd for NestedId {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.0[0].partial_cmp(&other.0[0])
    }
}

impl Ord for NestedId {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

pub struct UniqueId {
    pub(super) id: u32,
    pub(super) pid_ns: Weak<PidNamespace>,
}

impl UniqueId {
    pub fn new(id: u32, pid_ns: &Arc<PidNamespace>) -> Self {
        Self {
            id,
            pid_ns: Arc::downgrade(pid_ns),
        }
    }
}

impl PartialEq for UniqueId {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Weak::ptr_eq(&self.pid_ns, &other.pid_ns)
    }
}

impl PartialOrd for UniqueId {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        debug_assert!(Weak::ptr_eq(&self.pid_ns, &other.pid_ns));
        self.id.partial_cmp(&other.id)
    }
}

impl fmt::Debug for UniqueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniqueId")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NestedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NestedId").field("ids", &self.0).finish()
    }
}
