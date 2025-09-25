// SPDX-License-Identifier: MPL-2.0

mod clone;
pub mod credentials;
mod execve;
mod exit;
mod kill;
mod namespace;
mod pid_file;
pub mod posix_thread;
#[expect(clippy::module_inception)]
mod process;
mod process_filter;
pub mod process_table;
mod process_vm;
mod program_loader;
pub mod rlimit;
pub mod signal;
mod stats;
mod status;
pub mod sync;
mod task_set;
mod term_status;
mod wait;
pub use clone::{clone_child, CloneArgs, CloneFlags};
pub use credentials::{Credentials, Gid, Uid};
pub use execve::do_execve;
pub use kill::{kill, kill_all, kill_group, tgkill};
pub use namespace::{
    nsproxy::{check_unsupported_ns_flags, ContextSetNsAdminApi, NsProxy, NsProxyBuilder},
    unshare::ContextUnshareAdminApi,
    user_ns::UserNamespace,
};
pub use pid_file::PidFile;
pub use process::{
    broadcast_signal_async, enqueue_signal_async, spawn_init_process, ExitCode, JobControl, Pgid,
    Pid, Process, ProcessGroup, ReapedChildrenStats, Session, Sid, Terminal,
};
pub use process_filter::ProcessFilter;
pub use program_loader::{check_executable_file, ProgramToLoad};
pub use rlimit::ResourceType;
pub use stats::collect_process_creation_count;
pub use term_status::TermStatus;
pub use wait::{do_wait, WaitOptions, WaitStatus};

use crate::context::Context;

pub(super) fn init() {
    posix_thread::futex::init();
    stats::init();
}

pub(super) fn init_on_each_cpu() {
    process::init_on_each_cpu();
}

pub(super) fn init_in_first_process(ctx: &Context) {
    process::init_in_first_process(ctx);
}
