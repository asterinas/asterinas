// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

bitflags! {
    /// Flags used by `renameat2(2)`.
    ///
    /// Reference: <https://man7.org/linux/man-pages/man2/rename.2.html>.
    pub struct RenameFlags: u32 {
        /// Fail with `EEXIST` if the destination path already exists.
        const NOREPLACE = 1 << 0;
        /// Atomically exchange the source and destination paths.
        const EXCHANGE  = 1 << 1;
        /// Create a whiteout at the source path during the rename.
        const WHITEOUT  = 1 << 2;
    }
}
