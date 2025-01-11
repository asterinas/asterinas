// SPDX-License-Identifier: MPL-2.0

use super::*;

/// The per-cpu run queue for the STOP scheduling class.
///
/// This is a singleton class, meaning that only one thread can be in this class at a time.
/// This is used for the most critical tasks, such as powering off and rebooting.
pub(super) struct StopClassRq {
    thread: Option<Arc<Task>>,
}

impl StopClassRq {
    pub fn new() -> Self {
        Self { thread: None }
    }
}

impl core::fmt::Debug for StopClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.thread.is_some() {
            write!(f, "Stop: occupied")?;
        } else {
            write!(f, "Stop: empty")?;
        }
        Ok(())
    }
}

impl SchedClassRq for StopClassRq {
    fn enqueue(&mut self, thread: Arc<Task>, _: Option<EnqueueFlags>) {
        if self.thread.replace(thread).is_some() {
            panic!("Multiple `stop` threads spawned")
        }
    }

    fn len(&mut self) -> usize {
        usize::from(!self.is_empty())
    }

    fn is_empty(&mut self) -> bool {
        self.thread.is_none()
    }

    fn pick_next(&mut self) -> Option<Arc<Task>> {
        self.thread.take()
    }

    fn update_current(&mut self, _: &CurrentRuntime, _: &SchedAttr, _flags: UpdateFlags) -> bool {
        // Stop threads has the lowest priority value. They should never be preempted.
        false
    }
}
