// SPDX-License-Identifier: MPL-2.0

use core::mem;

use super::read_val_from_user;
use crate::{
    prelude::*,
    util::{read_bytes_from_user, write_bytes_to_user},
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct IoVec {
    base: Vaddr,
    len: usize,
}

impl IoVec {
    pub const fn base(&self) -> Vaddr {
        self.base
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn read_from_user(&self, buffer: &mut [u8]) -> Result<()> {
        debug_assert!(buffer.len() == self.len);
        if buffer.len() != self.len {
            return_errno_with_message!(
                Errno::EINVAL,
                "the length of buffer is not equal to the length of io vec"
            );
        }

        read_bytes_from_user(self.base, buffer)
    }

    pub fn write_to_user(&self, buffer: &[u8]) -> Result<()> {
        debug_assert!(buffer.len() == self.len);
        if buffer.len() != self.len {
            return_errno_with_message!(
                Errno::EINVAL,
                "the length of buffer is not equal to the length of io vec"
            );
        }

        write_bytes_to_user(self.base, buffer)
    }
}

/// Iterator over user provided Io Vectors
#[derive(Debug, Clone)]
pub struct IoVecIter {
    start_addr: Vaddr,
    iovec_count: usize,
    current: usize,
}

impl IoVecIter {
    /// Creates a new `IoVecIter`
    pub const fn new(start_addr: Vaddr, iovec_count: usize) -> Self {
        Self {
            start_addr,
            iovec_count,
            current: 0,
        }
    }
}

impl Iterator for IoVecIter {
    type Item = Result<IoVec>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current < self.iovec_count {
            let addr = self.start_addr + self.current * mem::size_of::<IoVec>();
            self.current += 1;

            let res = read_val_from_user::<IoVec>(addr);
            match res {
                // Skip the io vec whose base is zero
                Ok(io_vec) if io_vec.base() == 0 => continue,
                _ => return Some(res),
            }
        }

        None
    }
}
