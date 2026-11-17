// SPDX-License-Identifier: MPL-2.0

//! Complete symbolic-link targets through the shared flat-data cache.
//!
//! The consumer accepts nonempty targets up to 4095 bytes. Type and length
//! checks precede the target buffer and content I/O. The inode descriptor checks
//! the physical layout. The VFS `Path`/`Mount` chain retains the filesystem. Direct
//! callers retain the inode and filesystem until reading and accepted I/O
//! complete. The inode owns its output cache and raw backend, which retains the
//! device. Only complete contents are checked for NUL and UTF-8, in that order.

use ostd::mm::VmWriter;

use super::ErofsInode;
use crate::{
    fs::file::InodeType,
    prelude::{Errno, Error, Result, String, vec},
};

const MAX_SYMLINK_TARGET_SIZE_BYTES: u64 = 4095;

impl ErofsInode {
    /// Returns the full target string without appending a NUL terminator.
    pub(in super::super) fn read_link(&self) -> Result<String> {
        if self.description().inode_type() != InodeType::SymLink {
            return Err(Error::new(Errno::EINVAL));
        }
        let size_bytes = self.description().size_bytes();
        if size_bytes == 0 {
            return Err(Error::with_message(
                Errno::EIO,
                "erofs symlink target is empty",
            ));
        }
        if size_bytes > MAX_SYMLINK_TARGET_SIZE_BYTES {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "erofs symlink target is too long",
            ));
        }
        let size_bytes = usize::try_from(size_bytes).map_err(|_| Error::new(Errno::EOVERFLOW))?;
        let mut target = vec![0; size_bytes];
        let copied = {
            let mut writer = VmWriter::from(target.as_mut_slice()).to_fallible();
            self.data_cache.as_vmo().read(0, &mut writer, None)?;
            size_bytes - writer.avail()
        };
        if copied != size_bytes {
            return Err(Error::with_message(
                Errno::EIO,
                "incomplete erofs symlink target",
            ));
        }
        if target.contains(&0) {
            return Err(Error::with_message(
                Errno::EIO,
                "erofs symlink target contains NUL",
            ));
        }
        String::from_utf8(target).map_err(|_| {
            Error::with_message(Errno::EOPNOTSUPP, "erofs symlink target is not UTF-8")
        })
    }
}
