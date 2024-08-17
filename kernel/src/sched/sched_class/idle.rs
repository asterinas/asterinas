// SPDX-License-Identifier: MPL-2.0

use super::*;

/// The per-cpu run queue for the IDLE scheduling class.
///
/// This run queue is used for the per-cpu idle thread, if any.
pub(super) struct IdleClassRq {
    thread: Option<Arc<Thread>>,
}

impl IdleClassRq {
    pub fn new() -> Self {
        Self { thread: None }
    }
}

impl core::fmt::Debug for IdleClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.thread.is_some() {
            write!(f, "Idle: occupied")?;
        } else {
            write!(f, "Idle: empty")?;
        }
        Ok(())
    }
}

impl SchedClassRq for IdleClassRq {
    fn enqueue(&mut self, thread: Arc<Thread>, _: Option<EnqueueFlags>) {
        let ptr = Arc::as_ptr(&thread);
        if let Some(t) = self.thread.replace(thread)
            && ptr != Arc::as_ptr(&t)
        {
            panic!("Multiple `idle` threads spawned")
        }
    }

    fn len(&mut self) -> usize {
        usize::from(!self.is_empty())
    }

    fn is_empty(&mut self) -> bool {
        self.thread.is_none()
    }

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        self.thread.clone()
    }

    fn update_current(&mut self, _: &CurrentRuntime, _: &SchedAttr, _flags: UpdateFlags) -> bool {
        // Idle threads has the greatest priority value. They should always be preempted.
        true
    }
}
