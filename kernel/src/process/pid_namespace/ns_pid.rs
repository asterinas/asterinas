// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use super::PidNamespace;
use crate::prelude::*;

/// An array of [`NsPid`]s representing all the [`NsPid`]s
/// that a task can have in its own PID namespace
/// as well as in each of its ancestor PID namespaces.
///
/// The [`NsPid`]s in this array are ordered starting
/// with the [`NsPid`] from the root PID namespace,
/// followed by the [`NsPid`]s from each successive child namespace,
/// down to the current namespace.
#[derive(Clone)]
pub struct AncestorNsPids(pub(super) Arc<VecDeque<NsPid>>);

impl PartialEq for AncestorNsPids {
    fn eq(&self, other: &Self) -> bool {
        // Two `AncestorNsPids` can be compared based on the ID in the init PID namespace.
        self.0[0] == other.0[0]
    }
}

impl Eq for AncestorNsPids {}

impl PartialOrd for AncestorNsPids {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.0[0].partial_cmp(&other.0[0])
    }
}

impl Ord for AncestorNsPids {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// An identifier for a task (such as a thread, process, process group, or session)
/// within a PID namespace.
///
/// This ID consists of a `TaskId` (`u32`) and a weak reference to the PID namespace.
pub(super) struct NsPid {
    pub(super) id: TaskId,
    pub(super) pid_ns: Weak<PidNamespace>,
}

impl PartialEq for NsPid {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Weak::ptr_eq(&self.pid_ns, &other.pid_ns)
    }
}

impl PartialOrd for NsPid {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        debug_assert!(Weak::ptr_eq(&self.pid_ns, &other.pid_ns));
        self.id.partial_cmp(&other.id)
    }
}

impl fmt::Debug for NsPid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniqueId")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AncestorNsPids {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NestedId").field("ids", &self.0).finish()
    }
}

/// The identifier type for threads, processes, process groups, and sessions.
pub type TaskId = u32;
