// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/version` file support, which provides
//! information about the kernel version.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_version.5.html>

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{FileOps, ProcFileBuilder},
        vfs::inode::Inode,
    },
    net::uts_ns::UtsName,
    prelude::*,
};

/// Represents the inode at `/proc/version`.
pub struct VersionFileOps;

impl VersionFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/version.c#L23>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for VersionFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        // Get information from `UtsName`.
        let sysname = UtsName::SYSNAME;
        let release = UtsName::RELEASE;
        let version = UtsName::VERSION;

        // Get information from compile-time environment variables.
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
