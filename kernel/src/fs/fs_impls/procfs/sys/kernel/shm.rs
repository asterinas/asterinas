// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

const SHMALL: u64 = u64::MAX - (1u64 << 24);
const SHMMAX: u64 = u64::MAX - (1u64 << 24);
const SHMMNI: u64 = 4096;

enum ShmSysctlField {
    All,
    Max,
    Mni,
}

struct ShmSysctlFileOps {
    field: ShmSysctlField,
}

impl ShmSysctlFileOps {
    fn new_inode(parent: Weak<dyn Inode>, field: ShmSysctlField) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/ipc/ipc_sysctl.c#L292>
        ProcFile::new(Self { field }, parent, mkmod!(a+r, u+w))
    }

    fn name(&self) -> &'static str {
        match self.field {
            ShmSysctlField::All => "shmall",
            ShmSysctlField::Max => "shmmax",
            ShmSysctlField::Mni => "shmmni",
        }
    }

    fn value(&self) -> u64 {
        match self.field {
            ShmSysctlField::All => SHMALL,
            ShmSysctlField::Max => SHMMAX,
            ShmSysctlField::Mni => SHMMNI,
        }
    }
}

impl ProcFileOps for ShmSysctlFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", self.value())?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!(
            "writing to `/proc/sys/kernel/{}` is not supported",
            self.name()
        );
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "writing to the System V shared-memory sysctl file is not supported"
        );
    }
}

/// Represents the inode at `/proc/sys/kernel/shmall`.
pub struct ShmAllFileOps;

impl ShmAllFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ShmSysctlFileOps::new_inode(parent, ShmSysctlField::All)
    }
}

/// Represents the inode at `/proc/sys/kernel/shmmax`.
pub struct ShmMaxFileOps;

impl ShmMaxFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ShmSysctlFileOps::new_inode(parent, ShmSysctlField::Max)
    }
}

/// Represents the inode at `/proc/sys/kernel/shmmni`.
pub struct ShmMniFileOps;

impl ShmMniFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ShmSysctlFileOps::new_inode(parent, ShmSysctlField::Mni)
    }
}
