// SPDX-License-Identifier: MPL-2.0

use core::fmt::Write;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    Process,
};

/// Represents the inode at `/proc/[pid]/stat`.
/// The fields are the same as the ones in `/proc/[pid]/status`. But the format is different.
/// See https://github.com/torvalds/linux/blob/ce1c54fdff7c4556b08f5b875a331d8952e8b6b7/fs/proc/array.c#L467
/// FIXME: Some fields are not implemented yet.
pub struct StatFileOps(Arc<Process>);

impl StatFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for StatFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let process = &self.0;
        let main_thread = process.main_thread().unwrap();
        let file_table = main_thread.as_posix_thread().unwrap().file_table();

        let mut stat_output = String::new();
        writeln!(
            stat_output,
            "{} {} {} {} {} {} {}",
            process.executable_path(),
            process.pid(),
            process.pid(),
            process.parent().pid(),
            process.parent().pid(),
            file_table.lock().len(),
            process.tasks().lock().len()
        )
        .unwrap();
        Ok(stat_output.into_bytes())
    }
}
