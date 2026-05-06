// SPDX-License-Identifier: MPL-2.0

use super::PidDirOps;
use crate::{
    fs::{
        file::{InodeType, mkmod},
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
                DirOps, ListedEntry, ProcDir, ReaddirEntry, StaticDirEntry, keyed_readdir_entries,
                listed_entries_from_table, lookup_child_from_table, visit_listed_entries,
                visit_readdir_entries,
            },
        },
        vfs::inode::{Inode, RevalidationPolicy},
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

/// Represents the inode at `/proc/[pid]/task`.
pub struct TaskDirOps(Arc<PidEntry>);

impl TaskDirOps {
    pub fn new_inode(dir: &PidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3316>
        ProcDir::new(Self(dir.pid_entry().clone()), parent, mkmod!(a+rx))
    }

    fn process(&self) -> Option<Arc<Process>> {
        self.0.process_of_thread()
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]`.
#[derive(Clone)]
pub struct TidDirOps {
    pid_entry: Arc<PidEntry>,
}

impl TidDirOps {
    pub fn new(pid_entry: Arc<PidEntry>) -> Self {
        Self { pid_entry }
    }

    pub fn new_inode(pid_entry: Arc<PidEntry>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(
            Self { pid_entry },
            parent,
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3796>
            mkmod!(a+rx),
        )
    }

    pub fn pid_entry(&self) -> &Arc<PidEntry> {
        &self.pid_entry
    }

    pub(super) fn process(&self) -> Option<Arc<Process>> {
        self.pid_entry.process_of_thread()
    }

    pub(super) fn thread(&self) -> Option<Arc<Thread>> {
        self.pid_entry.thread()
    }

    pub(super) fn thread_and_process(&self) -> Option<(Arc<Thread>, Arc<Process>)> {
        let thread = self.thread()?;
        let process = thread.as_posix_thread().unwrap().weak_process().upgrade()?;

        Some((thread, process))
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [StaticDirEntry<
        fn(&TidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>,
    >] = &[
        ("auxv", InodeType::File, AuxvFileOps::new_inode),
        ("cgroup", InodeType::File, CgroupFileOps::new_inode),
        ("cmdline", InodeType::File, CmdlineFileOps::new_inode),
        ("comm", InodeType::File, CommFileOps::new_inode),
        ("environ", InodeType::File, EnvironFileOps::new_inode),
        ("exe", InodeType::SymLink, ExeSymOps::new_inode),
        ("fd", InodeType::Dir, FdDirOps::<fd::FileSymOps>::new_inode),
        (
            "fdinfo",
            InodeType::Dir,
            FdDirOps::<fd::FileInfoOps>::new_inode,
        ),
        ("gid_map", InodeType::File, GidMapFileOps::new_inode),
        ("mem", InodeType::File, MemFileOps::new_inode),
        ("mountinfo", InodeType::File, MountInfoFileOps::new_inode),
        ("mountstats", InodeType::File, MountStatsFileOps::new_inode),
        ("ns", InodeType::Dir, NsDirOps::new_inode),
        (
            "oom_score_adj",
            InodeType::File,
            OomScoreAdjFileOps::new_inode,
        ),
        ("stat", InodeType::File, StatFileOps::new_thread_inode),
        ("status", InodeType::File, StatusFileOps::new_inode),
        ("uid_map", InodeType::File, UidMapFileOps::new_inode),
        ("maps", InodeType::File, MapsFileOps::new_inode),
        ("mounts", InodeType::File, MountsFileOps::new_inode),
    ];
}

impl DirOps for TidDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.pid_entry().type_().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the thread or the process does not exist");
        };

        self.lookup_static_child(this_dir.this_weak().clone(), name)
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.pid_entry().type_().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the thread or the process does not exist");
        };

        visit_listed_entries(offset, self.static_listed_entries(), visit_fn)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        self.pid_entry().type_().is_some()
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

    pub(super) fn static_listed_entries(&self) -> impl Iterator<Item = ListedEntry<'_>> + '_ {
        listed_entries_from_table(Self::STATIC_ENTRIES)
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<Tid>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid TID");
        };

        // Note: After a PID-number recycling mechanism is introduced, there may be a race here:
        // - If a PID number is recycled as soon as its `PidEntry` is removed from the `PidTable`,
        //   then before the `PidTable` lock is acquired below, the current inode’s `PidEntry` may
        //   already have been removed and replaced with a new entry using the same PID number.
        //   In that case, it is possible that the main thread of the `Process` obtained here has not
        //   yet been deleted, causing `contains_tid` to return true even though we have actually
        //   retrieved the wrong `PidEntry`.
        // - If the PID number is not recycled until the `PidEntry` is dropped, then this race does not arise.
        //
        // TODO: Revisit and handle this potential race as appropriate once PID-number recycling is implemented.
        let Some(process) = self.process() else {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        };

        // Lock order: PID table -> tasks of process

        let pid_table = pid_table::pid_table_mut();

        let contains_tid = process
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .any(|task| task.as_posix_thread().unwrap().tid() == tid);
        if !contains_tid {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        }

        let Some(pid_entry) = pid_table.get_entry(tid) else {
            return_errno_with_message!(Errno::ENOENT, "the thread does not exist");
        };

        Ok(TidDirOps::new_inode(
            pid_entry,
            this_dir.this_weak().clone(),
        ))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        let Some(process) = self.process() else {
            return_errno_with_message!(Errno::ENOENT, "the process does not exist");
        };

        // Collect TIDs before visiting entries, as `visit_fn` may copy data to user memory.
        let tids = process
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .filter_map(|task| usize::try_from(task.as_posix_thread().unwrap().tid()).ok())
            .collect::<Vec<_>>();

        visit_readdir_entries(
            keyed_readdir_entries(offset, 2, tids, |tid| {
                ListedEntry::new(tid.to_string(), InodeType::Dir)
            }),
            visit_fn,
        )
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        if name.parse::<Tid>().is_err() {
            // For non-numeric names, the set of valid children does not change over time,
            // so we can rely on the cache without revalidating.
            return true;
        }

        if let Some(child) = child.downcast_ref::<ProcDir<TidDirOps>>() {
            // If the child's `PidEntry` still has the associated thread, it will not be removed
            // from the `PidTable` and remains alive.
            child.inner().pid_entry().type_().is_some()
        } else {
            false
        }
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        let Ok(tid) = name.parse::<Tid>() else {
            return true;
        };

        let Some(process) = self.process() else {
            return true;
        };

        process
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .all(|task| task.as_posix_thread().unwrap().tid() != tid)
    }
}
