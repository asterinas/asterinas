// SPDX-License-Identifier: MPL-2.0

use super::*;

#[derive(Debug)]
pub struct StopEntity(pub(super) ());

pub(super) struct StopClassRq {
    thread: Option<Arc<Thread>>,
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
    type Entity = StopEntity;

    fn enqueue(&mut self, thread: Arc<Thread>, _: &StopEntity) {
        if self.thread.replace(thread).is_some() {
            panic!("Multiple `stop` threads spawned")
        }
    }

    fn dequeue(&mut self, _: &StopEntity) {}

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        self.thread.take()
    }

    fn update_current(&mut self, _: &mut StopEntity, _flags: UpdateFlags) -> bool {
        self.thread.is_some()
    }
}

pub fn new_class(_cpu: u32) -> StopClassRq {
    StopClassRq { thread: None }
}
