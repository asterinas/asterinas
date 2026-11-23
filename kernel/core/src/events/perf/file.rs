// SPDX-License-Identifier: MPL-2.0

//! File descriptor implementation for counting `perf_event_open` events.
//!
//! Supports Linux's non-group `read(2)` layout and the `ENABLE`, `DISABLE`,
//! `RESET`, and `ID` `ioctl(2)` commands.

use core::fmt::Display;

use super::{
    EventSpec, PerfContext,
    context::{Operation, Ticket},
};
use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::Tid,
    util::ioctl::RawIoctl,
};

bitflags::bitflags! {
    /// Supported non-group flags in `perf_event_attr::read_format`.
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct ReadFormat: u64 {
        const TOTAL_TIME_ENABLED = 1 << 0;
        const TOTAL_TIME_RUNNING = 1 << 1;
        const ID = 1 << 2;
    }
}

/// An open `perf_event` file backed by a target thread's [`PerfContext`].
///
/// The file retains the context and a [`Ticket`] for its event,
/// so accumulated values remain readable after thread exit.
/// Descriptors sharing this file refer to the same event.
/// Dropping the last file reference closes the event and lets its owner CPU release any live counter.
pub(crate) struct PerfEventFile {
    common: FileCommon,
    context: Arc<PerfContext>,
    ticket: Ticket,
    read_format: ReadFormat,
    target_tid: Tid,
}

impl PerfEventFile {
    /// Creates a new `PerfEventFile` and attaches the event to `context`.
    ///
    /// `access_epoch` is the credential generation captured before authorizing the attachment.
    /// Attachment may wait for the target's owner CPU, so this method requires task context.
    pub(crate) fn new(
        context: Arc<PerfContext>,
        target_tid: Tid,
        spec: EventSpec,
        access_epoch: u64,
        read_format: ReadFormat,
    ) -> Result<Self> {
        // Allocate the anonymous inode path before attaching the event so that
        // no fallible step occurs after `context.create(spec)` succeeds.
        let path = AnonInodeFs::new_path(|_| "anon_inode:[perf_event]".to_string());
        let common = FileCommon::new(path, AccessMode::O_RDWR, StatusFlags::empty());
        let ticket = context.create(spec, access_epoch)?;
        Ok(Self {
            common,
            context,
            ticket,
            read_format,
            target_tid,
        })
    }
}

impl FileLike for PerfEventFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let word_count = 1 + self.read_format.bits().count_ones() as usize;
        let byte_len = word_count * size_of::<u64>();
        if writer.avail() < byte_len {
            return Err(errno!(ENOSPC, "perf read buffer is too small"));
        }

        let snapshot = self.context.read(self.ticket)?;

        // Write from the stack snapshot so user page faults never occur under PMU locks.
        writer.write_val(&snapshot.count)?;
        if self.read_format.contains(ReadFormat::TOTAL_TIME_ENABLED) {
            writer.write_val(&snapshot.enabled_ns)?;
        }
        if self.read_format.contains(ReadFormat::TOTAL_TIME_RUNNING) {
            writer.write_val(&snapshot.running_ns)?;
        }
        if self.read_format.contains(ReadFormat::ID) {
            writer.write_val(&self.ticket.id())?;
        }
        Ok(byte_len)
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        let operation = dispatch_ioctl!(match raw_ioctl {
            Enable => {
                Operation::Enable
            }
            Disable => {
                Operation::Disable
            }
            Reset => {
                Operation::Reset
            }
            cmd @ Id => {
                cmd.write(&self.ticket.id())?;
                return Ok(0);
            }
            _ => return Err(errno!(ENOTTY, "unsupported perf ioctl command")),
        });

        if raw_ioctl.arg() != 0 {
            return Err(errno!(EOPNOTSUPP, "perf group ioctl flags are unsupported"));
        }

        self.context.control(self.ticket, operation)?;
        Ok(0)
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        let mut flags = self.common.status_flags().bits() | self.common.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }
        Box::new(FdInfo {
            flags,
            id: self.ticket.id(),
            target_tid: self.target_tid,
        })
    }
}

impl Pollable for PerfEventFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl Drop for PerfEventFile {
    fn drop(&mut self) {
        self.context.close(self.ticket);
    }
}

mod ioctl_defs {
    use crate::{
        ioc,
        util::ioctl::{NoData, OutData},
    };

    pub(super) type Enable = ioc!(PERF_EVENT_IOC_ENABLE, b'$', 0, NoData);
    pub(super) type Disable = ioc!(PERF_EVENT_IOC_DISABLE, b'$', 1, NoData);
    pub(super) type Reset = ioc!(PERF_EVENT_IOC_RESET, b'$', 3, NoData);
    pub(super) type Id = ioc!(PERF_EVENT_IOC_ID, b'$', 7, OutData<u64>);
}

struct FdInfo {
    flags: u32,
    id: u64,
    target_tid: Tid,
}

impl Display for FdInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "pos:	0")?;
        writeln!(f, "flags:	0{:o}", self.flags)?;
        writeln!(f, "mnt_id:	{}", AnonInodeFs::mount_node().id())?;
        writeln!(f, "ino:	{}", AnonInodeFs::shared_inode().ino())?;
        writeln!(f, "event-id:	{}", self.id)?;
        writeln!(f, "pmu-target-tid:	{}", self.target_tid)
    }
}
