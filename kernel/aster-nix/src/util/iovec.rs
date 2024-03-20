// SPDX-License-Identifier: MPL-2.0

use super::read_val_from_user;
use crate::{
    prelude::*,
    util::{read_bytes_from_user, write_bytes_to_user},
};

/// A kernel space IO vector.
#[derive(Debug, Clone, Copy)]
pub struct IoVec {
    base: Vaddr,
    len: usize,
}

/// A user space IO vector.
///
/// The difference between `IoVec` and `UserIoVec`
/// is that `UserIoVec` uses `isize` as the length type,
/// while `IoVec` uses `usize`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct UserIoVec {
    base: Vaddr,
    len: isize,
}

impl TryFrom<UserIoVec> for IoVec {
    type Error = Error;

    fn try_from(value: UserIoVec) -> Result<Self> {
        if value.len < 0 {
            return_errno_with_message!(Errno::EINVAL, "the length of IO vector cannot be negative");
        }

        Ok(IoVec {
            base: value.base,
            len: value.len as usize,
        })
    }
}

impl IoVec {
    /// Creates a new `IoVec`.
    pub const fn new(base: Vaddr, len: usize) -> Self {
        Self { base, len }
    }

    /// Returns the base address.
    pub const fn base(&self) -> Vaddr {
        self.base
    }

    /// Returns the length.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the `IoVec` points to an empty user buffer.
    pub const fn is_empty(&self) -> bool {
        self.len == 0 || self.base == 0
    }

    /// Reads bytes from the user space buffer pointed by
    /// the `IoVec` to `dst`.
    ///
    /// If successful, the read length will be equal to `dst.len()`.
    ///
    /// # Panics
    ///
    /// This method will panic if
    /// 1.`dst.len()` is not the same as `self.len()`;
    /// 2. `self.is_empty()` is `true`.
    pub fn read_exact_from_user(&self, dst: &mut [u8]) -> Result<()> {
        assert_eq!(dst.len(), self.len);
        assert!(!self.is_empty());

        read_bytes_from_user(self.base, dst)
    }

    /// Writes bytes from the `src` buffer
    /// to the user space buffer pointed by the `IoVec`.
    ///
    /// If successful, the written length will be equal to `src.len()`.
    ///
    /// # Panics
    ///
    /// This method will panic if
    /// 1. `src.len()` is not the same as `self.len()`;
    /// 2. `self.is_empty()` is `true`.
    pub fn write_exact_to_user(&self, src: &[u8]) -> Result<()> {
        assert_eq!(src.len(), self.len);
        assert!(!self.is_empty());

        write_bytes_to_user(self.base, src)
    }

    /// Reads bytes to the `dst` buffer
    /// from the user space buffer pointed by the `IoVec`.
    ///
    /// If successful, returns the length of actually read bytes.
    pub fn read_from_user(&self, dst: &mut [u8]) -> Result<usize> {
        let len = self.len.min(dst.len());
        read_bytes_from_user(self.base, &mut dst[..len])?;
        Ok(len)
    }

    /// Writes bytes from the `src` buffer
    /// to the user space buffer pointed by the `IoVec`.
    ///
    /// If successful, returns the length of actually written bytes.
    pub fn write_to_user(&self, src: &[u8]) -> Result<usize> {
        let len = self.len.min(src.len());
        write_bytes_to_user(self.base, &src[..len])?;
        Ok(len)
    }
}

/// Copies IO vectors from user space.
pub fn copy_iovs_from_user(start_addr: Vaddr, count: usize) -> Result<Box<[IoVec]>> {
    let mut io_vecs = Vec::with_capacity(count);

    for idx in 0..count {
        let addr = start_addr + idx * core::mem::size_of::<UserIoVec>();
        let uiov = read_val_from_user::<UserIoVec>(addr)?;
        let iov = IoVec::try_from(uiov)?;
        io_vecs.push(iov);
    }

    Ok(io_vecs.into_boxed_slice())
}
