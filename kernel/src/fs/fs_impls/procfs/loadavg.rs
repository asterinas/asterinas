// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/loadavg` file support, which tells the user space
//! about the cpu load average for the last 1, 5, and 15 minutes.
//!
//! Reference: <https://www.man7.org/linux/man-pages/man5/proc_loadavg.5.html>

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::posix_thread,
    sched::{self, loadavg::get_loadavg},
};

/// Represents the inode at `/proc/loadavg`.
pub struct LoadAvgFileOps;

impl LoadAvgFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/loadavg.c#L33>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for LoadAvgFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let avg = get_loadavg();
        let (nr_queued, nr_running) = sched::nr_queued_and_running();
        writeln!(
            printer,
            "{:.2} {:.2} {:.2} {}/{} {}",
            avg[0],
            avg[1],
            avg[2],
            nr_running,
            nr_queued,
            posix_thread::last_tid(),
        )?;

        Ok(printer.bytes_written())
    }
}
