// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/version` file support, which provides
//! information about the kernel version.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_version.5.html>

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    net::UtsNamespace,
    prelude::*,
};

pub struct VersionFileOps;

impl VersionFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for VersionFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // Get UTS namespace information from the init namespace
        let uts_name = UtsNamespace::get_init_singleton().uts_name();

        let sysname = uts_name.sysname()?;
        let release = uts_name.release()?;
        let version = uts_name.version()?;

        // Get info from compile environment variables
        let compile_by = option_env!("OSDK_BUILD_USERNAME").unwrap_or("unknown");
        let compile_host = option_env!("OSDK_BUILD_HOSTNAME").unwrap_or("unknown");
        let compiler = option_env!("OSDK_BUILD_RUSTC").unwrap_or("unknown");

        // Reference:
        // <https://elixir.bootlin.com/linux/v6.17/source/init/version.c>
        // <https://elixir.bootlin.com/linux/v6.17/source/fs/proc/version.c>
        writeln!(
            printer,
            "{} version {} ({}@{}) ({}) {}",
            sysname, release, compile_by, compile_host, compiler, version
        )?;
        Ok(printer.bytes_written())
    }
}
