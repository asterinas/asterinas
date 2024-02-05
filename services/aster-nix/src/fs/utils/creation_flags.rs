// SPDX-License-Identifier: MPL-2.0

use bitflags::bitflags;

bitflags! {
    pub struct CreationFlags: u32 {
        /// create file if it does not exist
        const O_CREAT = 1 << 6;
        /// error if CREATE and the file exists
        const O_EXCL = 1 << 7;
        /// not become the process's controlling terminal
        const O_NOCTTY = 1 << 8;
        /// truncate file upon open
        const O_TRUNC = 1 << 9;
        /// file is a directory
        const O_DIRECTORY = 1 << 16;
        /// pathname is not a symbolic link
        const O_NOFOLLOW = 1 << 17;
        /// close on exec
        const O_CLOEXEC = 1 << 19;
        /// create an unnamed temporary regular file
        /// O_TMPFILE is (_O_TMPFILE | O_DIRECTORY)
        const _O_TMPFILE = 1 << 22;
    }
}
