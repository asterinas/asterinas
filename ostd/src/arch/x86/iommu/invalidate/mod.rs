// SPDX-License-Identifier: MPL-2.0

use queue::Queue;
use spin::Once;

use super::registers::{ExtendedCapabilityFlags, IOMMU_REGS};
use crate::{info, sync::SpinLock, warn};

pub mod descriptor;
pub mod queue;

pub(super) fn init() {
    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock();
    if !iommu_regs
        .read_extended_capability()
        .flags()
        .contains(ExtendedCapabilityFlags::QI)
    {
        warn!("Queued invalidation not supported");
        return;
    }

    QUEUE.call_once(|| {
        let queue = Queue::new();
        iommu_regs.enable_queued_invalidation(&queue);
        SpinLock::new(queue)
    });

    info!("Queued invalidation is enabled");
}

pub(super) static QUEUE: Once<SpinLock<Queue>> = Once::new();
