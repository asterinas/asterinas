use core::mem;

use inherit_methods_macro::inherit_methods;
use pod::Pod;

use crate::prelude::*;

use super::{VmAllocOptions, VmFrameVec, VmIo};

/// OwnedFrames represents one or more physical frames that we have exclusive access.
///
/// Once created, OwnedFrames has exclusive access to the specified physical frames,
/// achieved by prohibiting cloning to ensure that no other pointers can access these addresses.
///
/// OwnedFrames corresponds to contiguous physical pages and must be allocated continuously at once,
/// therefore we can implement the `as_slice` and `as_slice_mut` method for it.
pub struct OwnedFrames(VmFrameVec);

impl OwnedFrames {
    pub fn new(len: usize) -> Result<Self> {
        debug_assert!(len > 0);
        let frames = {
            let mut options = VmAllocOptions::new(len);
            options.is_contiguous(true);
            VmFrameVec::allocate(&options)?
        };
        Ok(Self(frames))
    }

    pub fn as_slice(&self) -> &[u8] {
        let data = self.start_addr() as *const u8;
        let len = self.0.nbytes();
        // Safety: no other pointers can write access this slice
        unsafe { core::slice::from_raw_parts(data, len) }
    }

    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        let data = self.start_addr() as *mut u8;
        let len = self.0.nbytes();
        // Safety: no other pointers can read/write access this slice
        unsafe { core::slice::from_raw_parts_mut(data, len) }
    }

    pub fn leak(self) -> &'static mut [u8] {
        let data = self.start_addr() as *mut u8;
        let len = self.0.nbytes();
        // Safety: we ensure the frames will never be deallocated, so it has exclusive access.
        let slice = unsafe { core::slice::from_raw_parts_mut(data, len) };
        mem::forget(self);
        slice
    }

    fn start_addr(&self) -> Vaddr {
        debug_assert!(self.0.len() > 0);
        self.0.get(0).unwrap().start_vaddr()
    }
}

#[inherit_methods(from = "self.0")]
impl VmIo for OwnedFrames {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()>;
    fn read_val<F: Pod>(&self, offset: usize) -> Result<F>;
    fn read_slice<F: Pod>(&self, offset: usize, slice: &mut [F]) -> Result<()>;
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()>;
    fn write_val<F: Pod>(&self, offset: usize, new_val: &F) -> Result<()>;
    fn write_slice<F: Pod>(&self, offset: usize, slice: &[F]) -> Result<()>;
}
