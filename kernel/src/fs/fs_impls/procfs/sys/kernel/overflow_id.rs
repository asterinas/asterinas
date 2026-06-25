// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::credentials::{Gid, Uid},
};

enum OverflowIdKind {
    Uid,
    Gid,
}

/// Represents `/proc/sys/kernel/overflowuid` and `/proc/sys/kernel/overflowgid`.
pub struct OverflowIdFileOps {
    kind: OverflowIdKind,
}

impl OverflowIdFileOps {
    pub fn new_uid_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        Self::new_inode(parent, OverflowIdKind::Uid)
    }

    pub fn new_gid_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        Self::new_inode(parent, OverflowIdKind::Gid)
    }

    fn new_inode(parent: Weak<dyn Inode>, kind: OverflowIdKind) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1720>
        ProcFile::new(Self { kind }, parent, mkmod!(a+r, u+w))
    }

    fn name(&self) -> &'static str {
        match self.kind {
            OverflowIdKind::Uid => "overflowuid",
            OverflowIdKind::Gid => "overflowgid",
        }
    }

    fn value(&self) -> u32 {
        match self.kind {
            OverflowIdKind::Uid => Uid::OVERFLOW.into(),
            OverflowIdKind::Gid => Gid::OVERFLOW.into(),
        }
    }
}

impl ProcFileOps for OverflowIdFileOps {
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
            "writing to the overflow UID/GID sysctl file is not supported"
        );
    }
}
