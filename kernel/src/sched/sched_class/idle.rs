// SPDX-License-Identifier: MPL-2.0

use super::*;

#[derive(Debug)]
pub struct IdleEntity(pub(super) ());

pub(super) struct IdleClassRq {
    thread: Option<Arc<Thread>>,
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
    type Entity = IdleEntity;

    fn enqueue(&mut self, thread: Arc<Thread>, _: &IdleEntity) {
        let ptr = Arc::as_ptr(&thread);
        if let Some(t) = self.thread.replace(thread)
            && ptr != Arc::as_ptr(&t)
        {
            panic!("Multiple `idle` threads spawned")
        }
    }

    fn dequeue(&mut self, _: &IdleEntity) {}

    fn pick_next(&mut self) -> Option<Arc<Thread>> {
        self.thread.clone()
    }

    fn update_current(&mut self, _: &mut IdleEntity, _flags: UpdateFlags) -> bool {
        true
    }
}

pub fn new_class(_cpu: u32) -> IdleClassRq {
    IdleClassRq { thread: None }
}
