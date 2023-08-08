extern crate alloc;

use core::fmt::Debug;
use core::marker::PhantomData;

use alloc::sync::Arc;
use jinux_frame::{
    io_mem::IoMem,
    vm::{HasPaddr, Paddr, VmFrame, VmIo},
    Result,
};
use pod::Pod;

#[derive(Debug, Clone)]
enum InFramePtrAccessMethod {
    IoMem(IoMem),
    VmFrame(Arc<VmFrame>),
}

impl InFramePtrAccessMethod {
    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        match self {
            InFramePtrAccessMethod::IoMem(mmio) => mmio.read_val(offset),
            InFramePtrAccessMethod::VmFrame(frame) => frame.read_val(offset),
        }
    }
}

/// An in-frame pointer to a POD value, enabling safe access
/// to a POD value given its physical memory address.
pub struct InFramePtr<T: 'static> {
    access_method: InFramePtrAccessMethod,
    offset: usize,
    marker: PhantomData<&'static mut T>,
}

impl<T: Debug + Pod + 'static> Debug for InFramePtr<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let inner = self.access_method.read_val::<T>(self.offset).unwrap();
        f.write_fmt(format_args!("{:?}", inner))
    }
}

impl<T: Pod> InFramePtr<T> {
    /// This function only allow the physical address in the MMIO region.
    ///
    /// Panic if the physical address is not in MMIO region.
    pub fn new(paddr: Paddr) -> Result<Self> {
        let limit = core::mem::size_of::<T>();
        Ok(Self {
            access_method: InFramePtrAccessMethod::IoMem(
                jinux_frame::io_mem::IoMem::new(paddr..paddr + limit).unwrap(),
            ),
            offset: 0,
            marker: PhantomData,
        })
    }

    /// Creating a pointer to the inside of VmFrame.
    pub fn new_with_vm_frame(vm_frame_vec: VmFrame) -> Result<Self> {
        Ok(Self {
            access_method: InFramePtrAccessMethod::VmFrame(Arc::new(vm_frame_vec)),
            offset: 0,
            marker: PhantomData,
        })
    }

    pub fn read(&self) -> T {
        self.access_method
            .read_val::<T>(self.offset)
            .expect("read inner from frame failed")
    }

    pub fn read_at<F: Pod>(&self, offset: *const F) -> F {
        match &self.access_method {
            InFramePtrAccessMethod::IoMem(mmio) => mmio
                .read_val::<F>(self.offset + offset as usize)
                .expect("write data from frame failed"),
            InFramePtrAccessMethod::VmFrame(vm_frame) => vm_frame
                .read_val::<F>(self.offset + offset as usize)
                .expect("write data from frame failed"),
        }
    }

    pub fn write_at<F: Pod>(&self, offset: *const F, new_val: F) {
        match &self.access_method {
            InFramePtrAccessMethod::IoMem(mmio) => mmio
                .write_val::<F>(self.offset + offset as usize, &new_val)
                .expect("write data from frame failed"),
            InFramePtrAccessMethod::VmFrame(vm_frame) => vm_frame
                .write_val::<F>(self.offset + offset as usize, &new_val)
                .expect("write data from frame failed"),
        }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn paddr(&self) -> usize {
        match &self.access_method {
            InFramePtrAccessMethod::IoMem(mmio) => self.offset + mmio.paddr(),
            InFramePtrAccessMethod::VmFrame(vm_frame) => self.offset + vm_frame.start_paddr(),
        }
    }

    /// Clone self and then change the offset to the next `count` one.
    ///
    /// User can use this function to easily visit POD array. For example:
    ///
    /// ```rust
    /// use pod::Pod
    ///
    /// #[derive(Pod)]
    /// struct Foo{
    ///     value1: usize,
    ///     value2: usize,
    /// }
    ///
    /// fn visit(){
    ///     // visit array [Foo1, Foo2, Foo3]
    ///     let Foo1 : InFramePtr<Foo> = InFramePtr::alloc().unwrap();
    ///     let Foo2 = Foo1.add(1);
    ///     let Foo3 = Foo2.add(1);
    /// }
    ///
    /// ```
    ///
    pub fn add(&self, count: usize) -> Self {
        let mut next: InFramePtr<T> = self.clone();
        next.access_method = match next.access_method {
            InFramePtrAccessMethod::IoMem(mmio) => InFramePtrAccessMethod::IoMem(
                jinux_frame::io_mem::IoMem::new(
                    mmio.paddr() + count * core::mem::size_of::<T>()
                        ..mmio.paddr() + (count + 1) * core::mem::size_of::<T>(),
                )
                .unwrap(),
            ),
            InFramePtrAccessMethod::VmFrame(_) => {
                next.offset += core::mem::size_of::<T>() * count;
                next.access_method
            }
        };
        next
    }
}

impl<T: Pod> Clone for InFramePtr<T> {
    fn clone(&self) -> Self {
        Self {
            access_method: self.access_method.clone(),
            offset: self.offset,
            marker: self.marker,
        }
    }
}
