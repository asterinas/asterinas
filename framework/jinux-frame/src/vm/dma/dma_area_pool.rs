use alloc::{sync::Arc, vec, vec::Vec};

use crate::{
    sync::SpinLock,
    vm::{VmAllocOptions, VmFrameVec},
    Result,
};

use super::dma_area::DmaArea;

/// `DmaAreaPool` maintains a memory pool for dma incoherent mapping.
/// For devices that require frequent allocation and deallocation of `DmaArea`,
/// using `DmaAreaPool` is a better choice.
///
/// `DmaAreaPool` maintains an array of `VmFrameVec`, and uses a first-fit strategy
/// to find the first VmFrameVec that satisfies the length requirement and splits the
/// corresponding pages for constructing a `DmaArea`.
///
/// `DmaAreaPool` is capable of dynamically adjusting the size of the memory pool.
/// When there are insufficient pages, it will pre-allocate pages in batches (called "expand"); when it has
/// excess pages, it will free some pages (called "contract").
pub struct DmaAreaPool {
    /// The batch size of pre-allocated pages
    page_batch: usize,
    /// The upper limit of free pages
    free_page_limit: usize,
    free_frame_pool: SpinLock<Vec<VmFrameVec>>,
}

impl DmaAreaPool {
    pub fn new() -> Result<Arc<Self>> {
        let page_batch = 8;
        let frame_vec = VmFrameVec::allocate(VmAllocOptions::new(page_batch).is_contiguous(true))?;
        Ok(Arc::new(Self {
            page_batch,
            free_page_limit: 32,
            free_frame_pool: SpinLock::new(vec![frame_vec]),
        }))
    }

    pub fn alloc(self: &Arc<Self>) -> Result<DmaArea> {
        if self.free_frame_pool.lock_irq_disabled().is_empty() {
            self.expand(self.page_batch)?;
        }

        let vm_frame_vec = self.lookup_frame_vec(1).unwrap();
        Ok(DmaArea::new(vm_frame_vec, Arc::downgrade(self)))
    }

    pub fn alloc_continuous(self: &Arc<Self>, count: usize) -> Result<DmaArea> {
        if self
            .free_frame_pool
            .lock_irq_disabled()
            .iter()
            .all(|vm_frame_vec| vm_frame_vec.len() < count)
        {
            self.expand(count.max(self.page_batch))?;
        }

        let vm_frame_vec = self.lookup_frame_vec(count).unwrap();
        Ok(DmaArea::new(vm_frame_vec, Arc::downgrade(self)))
    }

    pub(super) fn free(&self, vm_frame_vec: VmFrameVec) {
        self.free_frame_pool.lock_irq_disabled().push(vm_frame_vec);
        if self.num_frames() > self.free_page_limit {
            self.contract()
        }
    }

    fn num_frames(&self) -> usize {
        self.free_frame_pool
            .lock_irq_disabled()
            .iter()
            .map(|vm_frame_vec| vm_frame_vec.len())
            .sum()
    }

    /// Search for the appropriate VmFrameVec from the pool.
    /// Currently, we are utilizing the first-fit strategy to reduce latency.
    fn lookup_frame_vec(&self, count: usize) -> Option<VmFrameVec> {
        for vm_frame_vec in self.free_frame_pool.lock_irq_disabled().iter_mut() {
            if vm_frame_vec.len() > count {
                let mut result = VmFrameVec::empty();
                for _ in 0..count {
                    result.push(vm_frame_vec.pop().unwrap());
                }
                return Some(result);
            }
        }
        None
    }

    pub fn free_page_limit(&mut self, free_page_limit: usize) {
        self.free_page_limit = free_page_limit;
    }

    pub fn page_batch(&mut self, page_batch: usize) {
        self.page_batch = page_batch;
    }

    fn expand(&self, count: usize) -> Result<()> {
        let frame_vec = VmFrameVec::allocate(VmAllocOptions::new(count).is_contiguous(true))?;
        self.free_frame_pool.lock_irq_disabled().push(frame_vec);
        Ok(())
    }

    fn contract(&self) {
        let mut num_frames = self.num_frames();
        let mut free_frame_pool = self.free_frame_pool.lock_irq_disabled();
        while num_frames > self.free_page_limit {
            num_frames -= free_frame_pool.pop().unwrap().len();
        }
    }
}
