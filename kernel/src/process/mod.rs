// SPDX-License-Identifier: MPL-2.0

mod clone;
pub mod credentials;
mod exit;
mod kill;
mod pid_namespace;
pub mod posix_thread;
#[expect(clippy::module_inception)]
mod process;
mod process_filter;
mod process_vm;
mod program_loader;
pub mod rlimit;
pub mod signal;
mod status;
pub mod sync;
mod task_set;
mod term_status;
mod wait;

pub use clone::{clone_child, CloneArgs, CloneFlags};
pub use credentials::{Credentials, Gid, Uid};
pub use kill::{kill, kill_all, kill_group, tgkill};
pub use pid_namespace::{get_init_pid_namespace, PidEvent, PidNamespace, TASK_LIST_LOCK};
pub use process::{
    spawn_init_process, AsCurrentProcess, CurrentProcess, ExitCode, JobControl, Pgid, Pid, Process,
    ProcessGroup, Session, Sid, Terminal,
};
pub use process_filter::ProcessFilter;
pub use process_vm::{
    renew_vm_and_map, MAX_ARGV_NUMBER, MAX_ARG_LEN, MAX_ENVP_NUMBER, MAX_ENV_LEN,
};
pub use program_loader::{check_executable_file, ProgramToLoad};
pub use rlimit::ResourceType;
pub use term_status::TermStatus;
pub use wait::{wait_child_exit, WaitOptions};

pub(super) fn init() {
    process::init();
    posix_thread::futex::init();
    pid_namespace::init();
}
