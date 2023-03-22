use core::marker::PhantomData;

use jinux_frame::{
    config::PAGE_SIZE,
    vm::{Paddr, VmAllocOptions, VmFrame, VmFrameVec, VmIo},
    Result,
};
use pod::Pod;

/// An in-frame pointer to a POD value, enabling safe access
/// to a POD value given its physical memory address.
#[derive(Debug)]
pub struct InFramePtr<T: 'static> {
    frame: VmFrame,
    offset: usize,
    marker: PhantomData<&'static mut T>,
}

impl<T: Pod> InFramePtr<T> {
    pub fn new(paddr: Paddr) -> Result<Self> {
        let frame = {
            let page_paddr = paddr & !(PAGE_SIZE - 1);
            let mut options = VmAllocOptions::new(1);
            options.paddr(Some(page_paddr));
            VmFrameVec::allocate(&options)?.remove(0)
        };
        let offset = paddr - frame.start_paddr();
        Ok(Self {
            frame,
            offset,
            marker: PhantomData,
        })
    }

    pub fn read_at<F: Pod>(&self, offset: *const F) -> F {
        self.frame
            .read_val::<F>(self.offset + offset as usize)
            .expect("read data from frame failed")
    }

    pub fn write_at<F: Pod>(&self, offset: *const F, new_val: F) {
        self.frame
            .write_val::<F>(self.offset + offset as usize, &new_val)
            .expect("write data from frame failed");
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn paddr(&self) -> usize {
        self.offset + self.frame.start_paddr()
    }
}
