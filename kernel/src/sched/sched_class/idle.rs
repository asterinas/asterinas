// SPDX-License-Identifier: MPL-2.0

use super::*;

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
        let ptr = Arc::as_ptr(&entity);
        if let Some(t) = self.entity.replace(entity)
            && ptr != Arc::as_ptr(&t)
        {
            panic!("Multiple `idle` entities spawned")
        }
    }

    fn len(&mut self) -> usize {
        usize::from(!self.is_empty())
    }

    fn is_empty(&mut self) -> bool {
        self.entity.is_none()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        self.entity.clone()
    }

    fn update_current(&mut self, _: &CurrentRuntime, _: &SchedAttr, _flags: UpdateFlags) -> bool {
        // Idle entities has the greatest priority value. They should always be preempted.
        true
    }
}
