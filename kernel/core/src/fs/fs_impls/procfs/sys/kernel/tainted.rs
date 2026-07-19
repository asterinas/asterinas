// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps, read_u64_from},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/panic.h#L76>
const TAINT_FLAGS_COUNT: u64 = 20;
const TAINT_FLAGS_MASK: u64 = (1 << TAINT_FLAGS_COUNT) - 1;

static TAINTED_MASK: AtomicU64 = AtomicU64::new(0);

/// Represents the inode at `/proc/sys/kernel/tainted`.
pub struct TaintedFileOps;

impl TaintedFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1588>
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for TaintedFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", TAINTED_MASK.load(Ordering::Relaxed))?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            current_thread!().as_posix_thread().unwrap(),
            CapSet::SYS_ADMIN,
        ))?;

        let (mask, read_bytes) = read_u64_from(reader)?;

        TAINTED_MASK.fetch_or(mask & TAINT_FLAGS_MASK, Ordering::Relaxed);

        Ok(read_bytes)
    }
}
