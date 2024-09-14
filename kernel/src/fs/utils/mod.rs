// SPDX-License-Identifier: MPL-2.0

//! VFS components

pub use access_mode::AccessMode;
pub use channel::{Channel, Consumer, Producer};
pub use creation_flags::CreationFlags;
pub use dirent_visitor::DirentVisitor;
pub use direntry_vec::DirEntryVecExt;
pub use falloc_mode::FallocMode;
pub use file_creation_mask::FileCreationMask;
pub use flock::{FlockItem, FlockList, FlockType};
pub use fs::{FileSystem, FsFlags, SuperBlock};
pub use inode::{Extension, Inode, InodeMode, InodeType, Metadata, MknodType};
pub use ioctl::IoctlCmd;
pub use page_cache::{PageCache, PageCacheBackend};
pub use random_test::{generate_random_operation, new_fs_in_memory};
pub use range_lock::{
    FileRange, RangeLockItem, RangeLockItemBuilder, RangeLockList, RangeLockType, OFFSET_MAX,
};
pub use status_flags::StatusFlags;

mod access_mode;
mod channel;
mod creation_flags;
mod dirent_visitor;
mod direntry_vec;
mod falloc_mode;
mod file_creation_mask;
mod flock;
mod fs;
mod inode;
mod ioctl;
mod page_cache;
mod random_test;
mod range_lock;
mod status_flags;

use core::{
    borrow::Borrow,
    hash::{Hash, Hasher},
};

use crate::prelude::*;

#[derive(Copy, PartialEq, Eq, Clone, Debug)]
pub enum SeekFrom {
    Start(usize),
    End(isize),
    Current(isize),
}

/// Maximum bytes in a path
pub const PATH_MAX: usize = 4096;

/// Maximum bytes in a file name
pub const NAME_MAX: usize = 255;

/// The upper limit for resolving symbolic links
pub const SYMLINKS_MAX: usize = 40;

pub type CStr256 = FixedCStr<256>;
pub type Str16 = FixedStr<16>;
pub type Str64 = FixedStr<64>;

/// An owned C-compatible string with a fixed capacity of `N`.
///
/// The string is terminated with a null byte.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Pod)]
pub struct FixedCStr<const N: usize>([u8; N]);

impl<const N: usize> FixedCStr<N> {
    pub fn len(&self) -> usize {
        self.0.iter().position(|&b| b == 0).unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_str(&self) -> Result<&str> {
        Ok(alloc::str::from_utf8(self.as_bytes())?)
    }

    pub fn as_cstr(&self) -> Result<&CStr> {
        Ok(CStr::from_bytes_with_nul(self.as_bytes_with_nul())?)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0[0..self.len()]
    }

    pub fn as_bytes_with_nul(&self) -> &[u8] {
        &self.0[0..=self.len()]
    }
}

impl<'a, const N: usize> From<&'a [u8]> for FixedCStr<N> {
    fn from(bytes: &'a [u8]) -> Self {
        assert!(N > 0);

        let mut inner = [0u8; N];
        let len = {
            let mut nul_byte_idx = match bytes.iter().position(|&b| b == 0) {
                Some(idx) => idx,
                None => bytes.len(),
            };
            if nul_byte_idx >= N {
                nul_byte_idx = N - 1;
            }
            nul_byte_idx
        };
        inner[0..len].copy_from_slice(&bytes[0..len]);
        Self(inner)
    }
}

impl<'a, const N: usize> From<&'a str> for FixedCStr<N> {
    fn from(string: &'a str) -> Self {
        let bytes = string.as_bytes();
        Self::from(bytes)
    }
}

impl<'a, const N: usize> From<&'a CStr> for FixedCStr<N> {
    fn from(cstr: &'a CStr) -> Self {
        let bytes = cstr.to_bytes_with_nul();
        Self::from(bytes)
    }
}

impl<const N: usize> Default for FixedCStr<N> {
    fn default() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> Debug for FixedCStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.as_cstr() {
            Ok(cstr) => write!(f, "{:?}", cstr),
            Err(_) => write!(f, "{:?}", self.as_bytes()),
        }
    }
}

/// An owned string with a fixed capacity of `N`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Pod)]
pub struct FixedStr<const N: usize>([u8; N]);

impl<const N: usize> FixedStr<N> {
    pub fn len(&self) -> usize {
        self.0.iter().position(|&b| b == 0).unwrap_or(N)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_str(&self) -> Result<&str> {
        Ok(alloc::str::from_utf8(self.as_bytes())?)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0[0..self.len()]
    }
}

impl<'a, const N: usize> From<&'a [u8]> for FixedStr<N> {
    fn from(bytes: &'a [u8]) -> Self {
        let mut inner = [0u8; N];
        let len = {
            let mut nul_byte_idx = match bytes.iter().position(|&b| b == 0) {
                Some(idx) => idx,
                None => bytes.len(),
            };
            if nul_byte_idx > N {
                nul_byte_idx = N;
            }
            nul_byte_idx
        };
        inner[0..len].copy_from_slice(&bytes[0..len]);
        Self(inner)
    }
}

impl<'a, const N: usize> From<&'a str> for FixedStr<N> {
    fn from(string: &'a str) -> Self {
        let bytes = string.as_bytes();
        Self::from(bytes)
    }
}

impl<const N: usize> Borrow<[u8]> for FixedCStr<N> {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<const N: usize> Hash for FixedCStr<N> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl<const N: usize> Default for FixedStr<N> {
    fn default() -> Self {
        Self([0u8; N])
    }
}

impl<const N: usize> Debug for FixedStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.as_str() {
            Ok(string) => write!(f, "{}", string),
            Err(_) => write!(f, "{:?}", self.as_bytes()),
        }
    }
}
