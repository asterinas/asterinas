// SPDX-License-Identifier: MPL-2.0

//! The eBPF subsystem.
//!
//! This module implements the first phase of the roadmap in
//! <https://github.com/asterinas/asterinas/issues/3119>:
//!
//! - `BPF_PROG_LOAD`: verifies a user-supplied eBPF program and exposes it
//!   through a file descriptor.
//! - `BPF_LINK_CREATE`: attaches a loaded program to a Netfilter hook point
//!   (currently only the UDP send hook) and exposes the attachment through a
//!   file descriptor. Closing the descriptor detaches the program.
//!
//! The eBPF program is executed by the rbpf interpreter (no JIT), in line with
//! the roadmap.

mod link;
mod prog;
mod uapi;

pub(crate) use link::BpfLinkFile;
pub(crate) use prog::BpfProgFile;
pub(crate) use uapi::{
    AST_HOOK_UDP_SEND, BpfAttachType, BpfCmd, BpfProgType, bpf_link_create_attr, bpf_prog_load_attr,
};
