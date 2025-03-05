// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::task::{
    scheduler::{EnqueueFlags, UpdateFlags},
    Task,
};

use super::{CurrentRuntime, SchedAttr, SchedClassRq};

/// The per-cpu run queue for the STOP scheduling class.
///
/// This is a singleton class, meaning that only one thread can be in this class at a time.
/// This is used for the most critical tasks, such as powering off and rebooting.
pub(super) struct StopClassRq {
    entity: Option<Arc<Task>>,
}

impl StopClassRq {
    pub fn new() -> Self {
        Self { entity: None }
    }
}

impl core::fmt::Debug for StopClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.entity.is_some() {
            write!(f, "Stop: occupied")?;
        } else {
            write!(f, "Stop: empty")?;
        }
        Ok(())
    }
}

impl SchedClassRq for StopClassRq {
    fn enqueue(&mut self, entity: Arc<Task>, _: Option<EnqueueFlags>) {
        let old = self.entity.replace(entity);
        debug_assert!(
            old.is_none(),
            "the length of the stop queue should be no larger than 1"
        );
    }

    fn len(&self) -> usize {
        usize::from(!self.is_empty())
    }

    fn is_empty(&self) -> bool {
        self.entity.is_none()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        self.entity.take()
    }

    fn update_current(&mut self, _: &CurrentRuntime, _: &SchedAttr, _flags: UpdateFlags) -> bool {
        // Stop entities has the lowest priority value. They should never be preempted.
        false
    }
}
