// SPDX-License-Identifier: MPL-2.0

use crate::{
    mm::{FrameAllocOptions, PAGE_SIZE, Segment, VmIo},
    prelude::*,
};

pub(in crate::arch::iommu) struct Queue {
    segment: Segment<()>,
    queue_size: usize,
    tail: usize,
}

impl Queue {
    pub(in crate::arch::iommu) fn append_descriptor(&mut self, descriptor: u128) {
        if self.tail == self.queue_size {
            self.tail = 0;
        }
        self.segment
            .write_val(self.tail * size_of::<u128>(), &descriptor)
            .unwrap();
        self.tail += 1;
    }

    pub(in crate::arch::iommu) fn tail(&self) -> usize {
        self.tail
    }

    pub(in crate::arch::iommu) fn size(&self) -> usize {
        self.queue_size
    }

    pub(in crate::arch::iommu) fn base_paddr(&self) -> Paddr {
        self.segment.paddr()
    }

    pub(super) fn new() -> Self {
        const DEFAULT_PAGES: usize = 1;
        let segment = FrameAllocOptions::new()
            .alloc_segment(DEFAULT_PAGES)
            .unwrap();
        Self {
            segment,
            queue_size: (DEFAULT_PAGES * PAGE_SIZE) / size_of::<u128>(),
            tail: 0,
        }
    }
}
