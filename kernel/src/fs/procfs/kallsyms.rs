// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/kallsyms` file support, which provides
//! information about kernel symbols.
//!
//! https://man7.org/linux/man-pages/man5/proc_kallsyms.5.html

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
};

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
        let data = ostd::ksym::dump_ksyms()
            .map(|sym| sym.into_bytes())
            .unwrap_or(Vec::new());
        Ok(data)
    }
}
