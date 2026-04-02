// SPDX-License-Identifier: MPL-2.0

use super::PidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::{
            pid::task::{
                auxv::AuxvFileOps, cgroup::CgroupFileOps, cmdline::CmdlineFileOps,
                comm::CommFileOps, environ::EnvironFileOps, exe::ExeSymOps, fd::FdDirOps,
                gid_map::GidMapFileOps, maps::MapsFileOps, mem::MemFileOps,
                mountinfo::MountInfoFileOps, mounts::MountsFileOps, mountstats::MountStatsFileOps,
                ns::NsDirOps, oom_score_adj::OomScoreAdjFileOps, stat::StatFileOps,
                status::StatusFileOps, uid_map::UidMapFileOps,
            },
            template::{
                DirOps, ProcDir, ProcDirBuilder, ReaddirEntry, child_names_from_table,
                keyed_readdir_entries, lookup_child_from_table,
            },
        },
        vfs::inode::{Inode, RevalidateResult},
    },
    prelude::*,
    process::{Process, pid_table, pid_table::PidEntry, posix_thread::AsPosixThread},
    thread::{Thread, Tid},
};

mod auxv;
mod cgroup;
mod cmdline;
mod comm;
mod environ;
mod exe;
mod fd;
mod gid_map;
mod maps;
mod mem;
mod mountinfo;
mod mounts;
mod mountstats;
mod ns;
mod oom_score_adj;
pub(super) mod stat;
mod status;
mod uid_map;

pub(super) fn process_from_pid_entry(pid_entry: &PidEntry) -> Option<Arc<Process>> {
    pid_entry.process().or_else(|| {
        pid_entry
            .thread()
            .map(|thread| thread.as_posix_thread().unwrap().process())
    })
}

/// Represents the inode at `/proc/[pid]/task`.
pub struct TaskDirOps(Arc<PidEntry>);

impl TaskDirOps {
    pub fn new_inode(dir: &PidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3316>
        ProcDirBuilder::new(Self(dir.pid_entry().clone()), mkmod!(a+rx))
            .parent(parent)
            .need_revalidation()
            .need_neg_child_revalidation()
            .build()
            .unwrap()
    }

    fn process(&self) -> Option<Arc<Process>> {
        process_from_pid_entry(&self.0)
    }

    fn thread_entries(&self) -> Vec<(usize, String)> {
        let Some(process_ref) = self.process() else {
            return Vec::new();
        };

        let mut tids: Vec<_> = {
            let tasks = process_ref.tasks().lock();
            tasks
                .as_slice()
                .iter()
                .map(|task| task.as_posix_thread().unwrap().tid())
                .collect()
        };
        tids.sort_unstable();

        let pid_table = pid_table::pid_table_mut();
        tids.into_iter()
            .filter_map(|tid| {
                pid_table
                    .get_entry(tid)
                    .and_then(|_| usize::try_from(tid).ok())
            })
            .map(|tid| (tid, tid.to_string()))
            .collect()
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]`.
#[derive(Clone)]
pub(super) struct TidDirOps {
    pid_entry: Arc<PidEntry>,
}

impl TidDirOps {
    pub fn new(pid_entry: Arc<PidEntry>) -> Self {
        Self { pid_entry }
    }

    pub fn new_inode(pid_entry: Arc<PidEntry>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(
            Self { pid_entry },
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3796>
            mkmod!(a+rx),
        )
        .parent(parent)
        .need_revalidation()
        .need_neg_child_revalidation()
        .build()
        .unwrap()
    }

    pub(super) fn pid_entry(&self) -> &Arc<PidEntry> {
        &self.pid_entry
    }

    pub(super) fn process(&self) -> Option<Arc<Process>> {
        process_from_pid_entry(&self.pid_entry)
    }

    pub(super) fn thread(&self) -> Option<Arc<Thread>> {
        self.pid_entry.thread()
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(
        &'static str,
        fn(&TidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>,
    )] = &[
        ("auxv", AuxvFileOps::new_inode),
        ("cgroup", CgroupFileOps::new_inode),
        ("cmdline", CmdlineFileOps::new_inode),
        ("comm", CommFileOps::new_inode),
        ("environ", EnvironFileOps::new_inode),
        ("exe", ExeSymOps::new_inode),
        ("fd", FdDirOps::<fd::FileSymOps>::new_inode),
        ("fdinfo", FdDirOps::<fd::FileInfoOps>::new_inode),
        ("gid_map", GidMapFileOps::new_inode),
        ("mem", MemFileOps::new_inode),
        ("mountinfo", MountInfoFileOps::new_inode),
        ("mountstats", MountStatsFileOps::new_inode),
        ("ns", NsDirOps::new_inode),
        ("oom_score_adj", OomScoreAdjFileOps::new_inode),
        ("stat", StatFileOps::new_thread_inode),
        ("status", StatusFileOps::new_inode),
        ("uid_map", UidMapFileOps::new_inode),
        ("maps", MapsFileOps::new_inode),
        ("mounts", MountsFileOps::new_inode),
    ];
}

impl DirOps for TidDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        self.lookup_live_child(dir.this_weak().clone(), name)
    }

    fn child_names(&self, _dir: &ProcDir<Self>) -> Vec<String> {
        self.listed_child_names()
    }

    fn revalidate_pos_child(&self, _name: &str, _child: &dyn Inode) -> RevalidateResult {
        if self.process().is_none() || self.thread().is_none() {
            RevalidateResult::Invalid
        } else {
            RevalidateResult::Valid
        }
    }

    fn revalidate_neg_child(&self, name: &str) -> RevalidateResult {
        if Self::STATIC_ENTRIES
            .iter()
            .any(|(entry_name, _)| *entry_name == name)
        {
            RevalidateResult::Invalid
        } else {
            RevalidateResult::Valid
        }
    }
}

impl TidDirOps {
    pub(super) fn lookup_static_child(
        &self,
        this_ptr: Weak<dyn Inode>,
        name: &str,
    ) -> Result<Arc<dyn Inode>> {
        if let Some(child) =
            lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| (f)(self, this_ptr))
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    pub(super) fn lookup_live_child(
        &self,
        this_ptr: Weak<dyn Inode>,
        name: &str,
    ) -> Result<Arc<dyn Inode>> {
        if self.process().is_none() || self.thread().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the process or thread does not exist");
        }

        self.lookup_static_child(this_ptr, name)
    }

    pub(super) fn static_child_names(&self) -> Vec<String> {
        child_names_from_table(Self::STATIC_ENTRIES)
    }

    pub(super) fn listed_child_names(&self) -> Vec<String> {
        if self.process().is_none() || self.thread().is_none() {
            return Vec::new();
        }

        self.static_child_names()
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<Tid>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid TID");
        };

        let Some(process_ref) = self.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        };
        let contains_tid = process_ref
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .any(|task| task.as_posix_thread().unwrap().tid() == tid);
        if !contains_tid {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        }

        let pid_table = pid_table::pid_table_mut();
        let Some(pid_entry) = pid_table.get_entry(tid) else {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        };

        Ok(TidDirOps::new_inode(pid_entry, dir.this_weak().clone()))
    }

    fn child_names(&self, _dir: &ProcDir<Self>) -> Vec<String> {
        self.thread_entries()
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }

    fn entries_from_offset(&self, _dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        keyed_readdir_entries(offset, 2, self.thread_entries())
    }

    fn revalidate_pos_child(&self, name: &str, child: &dyn Inode) -> RevalidateResult {
        let Ok(tid) = name.parse::<Tid>() else {
            return RevalidateResult::Invalid;
        };
        let Some(child) = child.downcast_ref::<ProcDir<TidDirOps>>() else {
            return RevalidateResult::Invalid;
        };

        let Some(process_ref) = self.process() else {
            return RevalidateResult::Invalid;
        };
        let contains_tid = process_ref
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .any(|task| task.as_posix_thread().unwrap().tid() == tid);
        if !contains_tid {
            return RevalidateResult::Invalid;
        }

        let pid_table = pid_table::pid_table_mut();
        let Some(pid_entry) = pid_table.get_entry(tid) else {
            return RevalidateResult::Invalid;
        };

        if Arc::ptr_eq(&pid_entry, child.inner().pid_entry()) {
            RevalidateResult::Valid
        } else {
            RevalidateResult::Invalid
        }
    }

    fn revalidate_neg_child(&self, name: &str) -> RevalidateResult {
        let Ok(tid) = name.parse::<Tid>() else {
            return RevalidateResult::Valid;
        };

        let Some(process_ref) = self.process() else {
            return RevalidateResult::Valid;
        };

        if process_ref
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .all(|task| task.as_posix_thread().unwrap().tid() != tid)
        {
            RevalidateResult::Valid
        } else {
            RevalidateResult::Invalid
        }
    }
}
