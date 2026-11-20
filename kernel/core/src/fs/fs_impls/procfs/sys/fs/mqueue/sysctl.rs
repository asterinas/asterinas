// SPDX-License-Identifier: MPL-2.0

//! The sysctl files under `/proc/sys/fs/mqueue`.

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps, read_u64_from},
        vfs::inode::Inode,
    },
    ipc::mqueue::posix,
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

/// A sysctl file under `/proc/sys/fs/mqueue`.
#[derive(Clone, Copy, Debug)]
pub(super) enum MqueueSysctl {
    MsgDefault,
    MsgMax,
    MsgsizeDefault,
    MsgsizeMax,
    QueuesMax,
}

impl MqueueSysctl {
    pub(super) fn new_inode(self, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/ipc/mqueue.c#L2017>
        ProcFile::new(self, parent, mkmod!(a+r, u+w))
    }

    fn get(self) -> i64 {
        match self {
            Self::MsgDefault => posix::msg_default(),
            Self::MsgMax => posix::msg_max(),
            Self::MsgsizeDefault => posix::msgsize_default(),
            Self::MsgsizeMax => posix::msgsize_max(),
            Self::QueuesMax => posix::queues_max(),
        }
    }

    fn set(self, value: i64) -> Result<()> {
        match self {
            Self::MsgDefault => posix::set_msg_default(value),
            Self::MsgMax => posix::set_msg_max(value),
            Self::MsgsizeDefault => posix::set_msgsize_default(value),
            Self::MsgsizeMax => posix::set_msgsize_max(value),
            Self::QueuesMax => posix::set_queues_max(value),
        }
    }
}

impl ProcFileOps for MqueueSysctl {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", self.get())?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            UserNamespace::get_init_singleton().as_ref(),
            current_thread!().as_posix_thread().unwrap(),
            CapSet::SYS_ADMIN,
        ))?;

        let (value, read_bytes) = read_u64_from(reader)?;
        let value = i64::try_from(value).map_err(|_| errno!(EINVAL, "the value is too large"))?;
        self.set(value)?;

        Ok(read_bytes)
    }
}
