// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps, read_u64_from},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{Uid, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

/// Represents the inode at `/proc/sys/kernel/overflowuid`.
pub(super) struct OverflowUidFileOps;

impl OverflowUidFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1676>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for OverflowUidFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", u32::from(Uid::overflow()))?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            current_thread!().as_posix_thread().unwrap(),
            CapSet::SYS_ADMIN,
        ))?;

        let (value, read_bytes) = read_u64_from(reader)?;
        if value > super::MAX_OLDUID {
            return Err(Error::with_message(
                Errno::EINVAL,
                "the value is not a valid overflow UID",
            ));
        }

        Uid::set_overflow(Uid::new(value as u32));

        Ok(read_bytes)
    }
}
