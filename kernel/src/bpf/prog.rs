// SPDX-License-Identifier: MPL-2.0

//! The in-kernel representation of a loaded eBPF program.
//!
//! A [`BpfProgFile`] is a [`FileLike`] that owns verified eBPF bytecode. User
//! space holds a handle to it via a file descriptor returned by
//! `bpf(BPF_PROG_LOAD, ...)`; the kernel holds additional `Arc` references
//! through any [`BpfLinkFile`] that has attached the program to a hook.
//!
//! [`BpfLinkFile`]: super::BpfLinkFile

use core::fmt::Display;

use aster_bigtcp::netfilter::ebpf::EbpfHook;

use super::uapi::BpfProgType;
use crate::{
    events::IoEvents,
    fs::{
        file::{FileLike, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// A loaded eBPF program exposed as a kernel object.
pub(crate) struct BpfProgFile {
    prog_type: BpfProgType,
    hook: Arc<EbpfHook>,
    pseudo_path: Path,
}

impl BpfProgFile {
    pub(crate) fn new(prog_type: BpfProgType, bytecode: Vec<u8>) -> Result<Arc<Self>> {
        let hook = EbpfHook::new(bytecode).map_err(|_err| {
            Error::with_message(Errno::EINVAL, "the eBPF bytecode failed verification")
        })?;
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:bpf-prog".to_string());
        Ok(Arc::new(Self {
            prog_type,
            hook: Arc::new(hook),
            pseudo_path,
        }))
    }

    pub(crate) fn prog_type(&self) -> BpfProgType {
        self.prog_type
    }

    pub(crate) fn hook(&self) -> &Arc<EbpfHook> {
        &self.hook
    }
}

impl Pollable for BpfProgFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // BPF program FDs are passive objects; no IO readiness to report.
        IoEvents::empty() & mask
    }
}

impl FileLike for BpfProgFile {
    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            prog_type: BpfProgType,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                writeln!(f, "prog_type:\t{}", self.prog_type as u32)
            }
        }

        Box::new(FdInfo {
            prog_type: self.prog_type,
        })
    }
}
