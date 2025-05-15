// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use super::PidNamespace;
use crate::prelude::*;

/// An array of [`UniqueId`]s, representing all the [`UniqueId`]s a task can have.
///
/// The [`UniqueId`]s in this array are sorted,
/// starting with the [`UniqueId`] from the init PID namespace,
/// followed by the [`UniqueId`] from the child namespace of the init PID namespace, and so on.
#[derive(Clone)]
pub struct UniqueIdArray(pub(super) Arc<VecDeque<UniqueId>>);

impl PartialEq for UniqueIdArray {
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

impl Eq for UniqueIdArray {}

impl PartialOrd for UniqueIdArray {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        // Two `UniqueIdArray` can be compared based on the ID in the init PID namespace.
        self.0[0].partial_cmp(&other.0[0])
    }
}

impl Ord for UniqueIdArray {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// A unique ID for identifying a task.
///
/// A unique ID contains a `TaskId`(u32) and a weak reference to the PID namespace.
pub(super) struct UniqueId {
    pub(super) id: TaskId,
    pub(super) pid_ns: Weak<PidNamespace>,
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

impl fmt::Debug for UniqueIdArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NestedId").field("ids", &self.0).finish()
    }
}

/// The identifier type for threads, processes, process groups, and sessions.
pub type TaskId = u32;
