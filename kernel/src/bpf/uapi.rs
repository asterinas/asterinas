// SPDX-License-Identifier: MPL-2.0

//! BPF user-visible ABI constants and structures.
//!
//! The constants follow Linux UAPI values so standard toolchains such as
//! libbpf can be used. See <linux/bpf.h>.

use crate::prelude::*;

/// `bpf()` subcommand numbers.
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum BpfCmd {
    MapCreate = 0,
    MapLookupElem = 1,
    MapUpdateElem = 2,
    MapDeleteElem = 3,
    MapGetNextKey = 4,
    ProgLoad = 5,
    LinkCreate = 28,
}

/// Program types recognized by `BPF_PROG_LOAD`.
///
/// Only [`BpfProgType::Netfilter`] is supported in phase 1.
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum BpfProgType {
    Unspec = 0,
    Netfilter = 45,
}

/// Attach types recognized by `BPF_LINK_CREATE`.
///
/// Only [`BpfAttachType::Netfilter`] is supported in phase 1.
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum BpfAttachType {
    Netfilter = 45,
}

/// Asterinas-private hook number for the UDP send hook.
///
/// Linux's generic netfilter hooks (`NF_INET_*`) do not cleanly map onto the
/// single "before a UDP packet is handed off to the iface" hook that
/// `aster-bigtcp` exposes today. Rather than lie about emulating a Linux
/// hook, we expose the hook under a private identifier until the netfilter
/// hook coverage grows.
pub const AST_HOOK_UDP_SEND: u32 = 0x1000;

/// `BPF_PROG_LOAD` attribute layout.
///
/// Matches the first fields of the `BPF_PROG_LOAD` arm of `union bpf_attr`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
#[allow(non_camel_case_types)]
pub struct bpf_prog_load_attr {
    pub prog_type: u32,
    pub insn_cnt: u32,
    pub insns: u64,
    pub license: u64,
    pub log_level: u32,
    pub log_size: u32,
    pub log_buf: u64,
    pub kern_version: u32,
    pub prog_flags: u32,
    pub prog_name: [u8; 16],
    pub prog_ifindex: u32,
    pub expected_attach_type: u32,
}

/// `BPF_LINK_CREATE` attribute layout (netfilter attach variant).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
#[allow(non_camel_case_types)]
pub struct bpf_link_create_attr {
    pub prog_fd: u32,
    pub target_fd_or_ifindex: u32,
    pub attach_type: u32,
    pub flags: u32,
    /// For the netfilter arm: protocol family (`AF_INET` etc.).
    pub nf_pf: u32,
    /// For the netfilter arm: hook number.
    pub nf_hooknum: u32,
    /// For the netfilter arm: priority.
    pub nf_priority: i32,
    /// For the netfilter arm: flags.
    pub nf_flags: u32,
}
