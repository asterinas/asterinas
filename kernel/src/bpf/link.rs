// SPDX-License-Identifier: MPL-2.0

//! The in-kernel representation of a BPF link.
//!
//! A [`BpfLinkFile`] represents an attachment of a loaded eBPF program
//! to a hook point. Closing the file descriptor detaches the program from
//! the hook.

use core::fmt::Display;

use aster_bigtcp::netfilter::{self, HookContext, HookFunction, HookId, Verdict};

use super::prog::BpfProgFile;
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

/// The hook point at which a [`BpfLinkFile`] is attached.
#[derive(Clone, Copy, Debug)]
enum AttachPoint {
    /// The `UdpSocket::send` hook in `aster-bigtcp`.
    NetfilterUdpSend,
}

/// A BPF program attached to a hook point.
pub(crate) struct BpfLinkFile {
    // Keep the program alive for at least as long as the link exists; the
    // registry entry borrows the program's bytecode through the hook closure.
    _prog: Arc<BpfProgFile>,
    attach: AttachPoint,
    hook_id: HookId,
    pseudo_path: Path,
}

impl BpfLinkFile {
    /// Attaches `prog` to the netfilter UDP send hook.
    pub(crate) fn attach_netfilter_udp_send(prog: Arc<BpfProgFile>) -> Result<Arc<Self>> {
        let registry = netfilter::registry().ok_or_else(|| {
            Error::with_message(Errno::ENODEV, "the netfilter registry is not initialized")
        })?;

        let hook = Box::new(ProgHook { prog: prog.clone() });
        let hook_id = registry.register_udp_send_hook(hook);
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:bpf-link".to_string());

        Ok(Arc::new(Self {
            _prog: prog,
            attach: AttachPoint::NetfilterUdpSend,
            hook_id,
            pseudo_path,
        }))
    }
}

impl Drop for BpfLinkFile {
    fn drop(&mut self) {
        let Some(registry) = netfilter::registry() else {
            return;
        };
        match self.attach {
            AttachPoint::NetfilterUdpSend => {
                registry.unregister_udp_send_hook(self.hook_id);
            }
        }
    }
}

impl Pollable for BpfLinkFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        // Links are passive attachments; no IO readiness to report.
        IoEvents::empty() & mask
    }
}

impl FileLike for BpfLinkFile {
    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, _fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            attach: AttachPoint,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let name = match self.attach {
                    AttachPoint::NetfilterUdpSend => "netfilter_udp_send",
                };
                writeln!(f, "link_type:\t{}", name)
            }
        }

        Box::new(FdInfo {
            attach: self.attach,
        })
    }
}

/// The hook function that the netfilter registry sees.
///
/// It forwards the hook invocation to the eBPF program owned by the
/// referenced [`BpfProgFile`].
struct ProgHook {
    prog: Arc<BpfProgFile>,
}

impl HookFunction for ProgHook {
    fn run(&self, context: &mut HookContext) -> Option<Verdict> {
        self.prog.hook().run(context)
    }
}
