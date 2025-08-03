// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::task::{
    scheduler::{EnqueueFlags, UpdateFlags},
    Task,
};

use super::{CurrentRuntime, SchedAttr, SchedClassRq};

/// The per-cpu run queue for the IDLE scheduling class.
///
/// This run queue is used for the per-cpu idle entity, if any.
pub(super) struct IdleClassRq {
    entity: Option<Arc<Task>>,
}

impl IdleClassRq {
    pub fn new() -> Self {
        Self { entity: None }
    }
}

impl core::fmt::Debug for IdleClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.entity.is_some() {
            write!(f, "Idle: occupied")?;
        } else {
            write!(f, "Idle: empty")?;
        }
        Ok(())
    }
}

impl SchedClassRq for IdleClassRq {
    fn enqueue(&mut self, entity: Arc<Task>, _: Option<EnqueueFlags>) {
        let old = self.entity.replace(entity);
        debug_assert!(
            old.is_none(),
            "the length of the idle queue should be no larger than 1"
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
        !self.is_empty()
    }
}
