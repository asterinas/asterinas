// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    net::uts_ns::UtsName,
    prelude::*,
};

enum UtsSysctlField {
    OsType,
    OsRelease,
    Version,
}

struct UtsSysctlFileOps {
    field: UtsSysctlField,
}

impl UtsSysctlFileOps {
    fn new_inode(parent: Weak<dyn Inode>, field: UtsSysctlField) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1765>
        ProcFile::new(Self { field }, parent, mkmod!(a+r))
    }

    fn value(&self) -> &'static str {
        match self.field {
            UtsSysctlField::OsType => UtsName::SYSNAME,
            UtsSysctlField::OsRelease => UtsName::RELEASE,
            UtsSysctlField::Version => UtsName::VERSION,
        }
    }
}

impl ProcFileOps for UtsSysctlFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", self.value())?;

        Ok(printer.bytes_written())
    }
}

/// Represents the inode at `/proc/sys/kernel/ostype`.
pub struct OsTypeFileOps;

impl OsTypeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        UtsSysctlFileOps::new_inode(parent, UtsSysctlField::OsType)
    }
}

/// Represents the inode at `/proc/sys/kernel/osrelease`.
pub struct OsReleaseFileOps;

impl OsReleaseFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        UtsSysctlFileOps::new_inode(parent, UtsSysctlField::OsRelease)
    }
}

/// Represents the inode at `/proc/sys/kernel/version`.
pub struct VersionFileOps;

impl VersionFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        UtsSysctlFileOps::new_inode(parent, UtsSysctlField::Version)
    }
}
