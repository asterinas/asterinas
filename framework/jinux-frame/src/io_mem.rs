use core::{mem::size_of, ops::Range};
use pod::Pod;
use spin::Once;

use crate::{
    vm::{HasPaddr, Paddr, Vaddr, VmIo},
    Error,
};

static CHECKER: Once<MmioChecker> = Once::new();

pub(crate) fn init() {
    CHECKER.call_once(|| MmioChecker {
        start: crate::arch::mmio::start_address(),
        end: crate::arch::mmio::end_address(),
    });
    log::info!(
        "MMIO start: 0x{:x}, end: 0x{:x}",
        CHECKER.get().unwrap().start,
        CHECKER.get().unwrap().end
    );
}

#[derive(Debug, Clone)]
pub struct IoMem {
    virtual_address: Vaddr,
    limit: usize,
}

impl VmIo for IoMem {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> crate::Result<()> {
        self.check_range(offset, buf.len())?;
        unsafe {
            core::ptr::copy(
                self.virtual_address as *const u8,
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> crate::Result<()> {
        self.check_range(offset, buf.len())?;
        unsafe {
            core::ptr::copy(buf.as_ptr(), self.virtual_address as *mut u8, buf.len());
        }
        Ok(())
    }

    fn read_val<T: Pod>(&self, offset: usize) -> crate::Result<T> {
        self.check_range(offset, size_of::<T>())?;
        Ok(unsafe { core::ptr::read_volatile((self.virtual_address + offset) as *const T) })
    }

    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> crate::Result<()> {
        self.check_range(offset, size_of::<T>())?;
        unsafe { core::ptr::write_volatile((self.virtual_address + offset) as *mut T, *new_val) };
        Ok(())
    }
}

impl HasPaddr for IoMem {
    fn paddr(&self) -> Paddr {
        crate::vm::vaddr_to_paddr(self.virtual_address).unwrap()
    }
}

impl IoMem {
    pub fn new(range: Range<Paddr>) -> Option<IoMem> {
        if CHECKER.get().unwrap().check(&range) {
            Some(IoMem {
                virtual_address: crate::vm::paddr_to_vaddr(range.start),
                limit: range.len(),
            })
        } else {
            None
        }
    }

    fn check_range(&self, offset: usize, len: usize) -> crate::Result<()> {
        let sum = offset.checked_add(len).ok_or(Error::InvalidArgs)?;
        if sum > self.limit {
            log::error!(
                "attempt to access address out of bounds, limit:0x{:x}, access position:0x{:x}",
                self.limit,
                sum
            );
            Err(Error::InvalidArgs)
        } else {
            Ok(())
        }
    }
}

struct MmioChecker {
    start: Paddr,
    end: Paddr,
}

impl MmioChecker {
    /// Check whether the physical address is in MMIO region.
    fn check(&self, range: &Range<Paddr>) -> bool {
        range.start >= self.start && range.end < self.end
    }
}
