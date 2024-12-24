// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use super::*;

/// The per-cpu run queue for the STOP scheduling class.
///
/// This is a singleton class, meaning that only one thread can be in this class at a time.
/// This is used for the most critical tasks, such as powering off and rebooting.
pub(super) struct StopClassRq {
    has_value: AtomicBool,
    entity: SpinLock<Option<SchedEntity>>,
}

impl StopClassRq {
    pub fn new() -> Arc<Self> {
        Arc::new(StopClassRq {
            has_value: AtomicBool::new(false),
            entity: SpinLock::new(None),
        })
    }
}

impl core::fmt::Debug for StopClassRq {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.has_value.load(Relaxed) {
            write!(f, "Stop: occupied")?;
        } else {
            write!(f, "Stop: empty")?;
        }
        Ok(())
    }
}

impl SchedClassRq for Arc<StopClassRq> {
    fn enqueue(&mut self, entity: SchedEntity, _: Option<EnqueueFlags>) {
        let mut lock = self.entity.lock();
        if lock.replace(entity).is_some() {
            panic!("Multiple `stop` threads spawned")
        }
        self.has_value.store(true, Relaxed);
    }

    fn len(&mut self) -> usize {
        usize::from(!self.is_empty())
    }

    fn is_empty(&mut self) -> bool {
        !self.has_value.load(Relaxed)
    }

    fn pick_next(&mut self) -> Option<SchedEntity> {
        let mut lock = self.entity.lock();
        self.has_value.store(false, Relaxed);
        lock.take()
    }

    fn update_current(&mut self, _: &CurrentRuntime, _: &SchedAttr, _flags: UpdateFlags) -> bool {
        // Stop threads has the lowest priority value. They should never be preempted.
        false
    }
}
