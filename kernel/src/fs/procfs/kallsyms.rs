// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/kallsyms` file support, which provides
//! information about kernel symbols.
//!
//! https://man7.org/linux/man-pages/man5/proc_kallsyms.5.html

use spin::Once;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
};

static KALLSYMS: Once<Vec<u8>> = Once::new();

pub(crate) fn init_kernel_symbols(kallsyms: Vec<u8>) {
    KALLSYMS.call_once(|| kallsyms);
}

/// File operations for `/proc/kallsyms`.
pub struct KallsymsFileOps;

impl KallsymsFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for KallsymsFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let symbols = KALLSYMS.get().cloned();
        Ok(symbols.unwrap_or_else(|| vec![]))
    }
}
