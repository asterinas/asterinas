// SPDX-License-Identifier: MPL-2.0

//! The `bpf()` system call.
//!
//! Phase 1 of <https://github.com/asterinas/asterinas/issues/3119> only
//! supports two subcommands:
//!
//! - `BPF_PROG_LOAD`: load an eBPF program.
//! - `BPF_LINK_CREATE`: attach a loaded program to a Netfilter hook.

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    bpf::{
        AST_HOOK_UDP_SEND, BpfAttachType, BpfCmd, BpfLinkFile, BpfProgFile, BpfProgType,
        bpf_link_create_attr, bpf_prog_load_attr,
    },
    fs::file::file_table::FdFlags,
    prelude::*,
    util::CopyCompat,
};

/// The maximum number of eBPF instructions accepted by the verifier.
///
/// Matches the Linux `BPF_MAXINSNS` constant so off-the-shelf programs load
/// unchanged.
const MAX_INSNS: u32 = 4096;

/// The size of a single eBPF instruction in bytes.
const INSN_SIZE: u32 = 8;

pub fn sys_bpf(cmd: u32, attr_addr: Vaddr, size: u32, ctx: &Context) -> Result<SyscallReturn> {
    let cmd = BpfCmd::try_from(cmd)
        .map_err(|_| Error::with_message(Errno::EINVAL, "unknown bpf command"))?;
    debug!("bpf command = {:?}, attr_size = {}", cmd, size);

    match cmd {
        BpfCmd::ProgLoad => sys_bpf_prog_load(attr_addr, size as usize, ctx),
        BpfCmd::LinkCreate => sys_bpf_link_create(attr_addr, size as usize, ctx),
        _ => return_errno_with_message!(Errno::EINVAL, "bpf command is not supported"),
    }
}

fn sys_bpf_prog_load(attr_addr: Vaddr, size: usize, ctx: &Context) -> Result<SyscallReturn> {
    let attr = ctx
        .user_space()
        .read_val_compat::<bpf_prog_load_attr>(attr_addr, size)?;

    let prog_type = BpfProgType::try_from(attr.prog_type).map_err(|_| {
        Error::with_message(Errno::EINVAL, "the eBPF program type is not supported")
    })?;

    if !matches!(prog_type, BpfProgType::Netfilter) {
        return_errno_with_message!(
            Errno::EINVAL,
            "only BPF_PROG_TYPE_NETFILTER is supported in phase 1"
        );
    }

    if attr.insn_cnt == 0 || attr.insn_cnt > MAX_INSNS {
        return_errno_with_message!(Errno::E2BIG, "the eBPF instruction count is out of range");
    }

    let bytecode_len = attr.insn_cnt.checked_mul(INSN_SIZE).ok_or_else(|| {
        Error::with_message(Errno::E2BIG, "the eBPF instruction count is out of range")
    })? as usize;
    let mut bytecode = vec![0u8; bytecode_len];
    ctx.user_space()
        .read_bytes(attr.insns as Vaddr, &mut bytecode)?;

    let prog = BpfProgFile::new(prog_type, bytecode)?;

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd = file_table_locked.insert(prog, FdFlags::empty());
    Ok(SyscallReturn::Return(fd.into()))
}

fn sys_bpf_link_create(attr_addr: Vaddr, size: usize, ctx: &Context) -> Result<SyscallReturn> {
    let attr = ctx
        .user_space()
        .read_val_compat::<bpf_link_create_attr>(attr_addr, size)?;

    let attach_type = BpfAttachType::try_from(attr.attach_type)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the eBPF attach type is not supported"))?;

    // The phase 1 roadmap only targets the UDP send hook, which we expose
    // through an Asterinas-private `hooknum` until we grow proper coverage
    // of the Linux `NF_INET_*` hook points.
    if !matches!(attach_type, BpfAttachType::Netfilter) || attr.nf_hooknum != AST_HOOK_UDP_SEND {
        return_errno_with_message!(
            Errno::EINVAL,
            "only the Asterinas UDP send netfilter hook is supported"
        );
    }

    let file = {
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.unwrap().read();
        file_table_locked
            .get_file((attr.prog_fd as i32).try_into()?)?
            .clone()
    };
    let prog: Arc<BpfProgFile> = Arc::downcast(file)
        .map_err(|_| Error::with_message(Errno::EBADF, "the fd is not a bpf program"))?;

    if !matches!(prog.prog_type(), BpfProgType::Netfilter) {
        return_errno_with_message!(
            Errno::EINVAL,
            "the program type does not match the attach type"
        );
    }

    let link = BpfLinkFile::attach_netfilter_udp_send(prog)?;

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd = file_table_locked.insert(link, FdFlags::empty());
    Ok(SyscallReturn::Return(fd.into()))
}
