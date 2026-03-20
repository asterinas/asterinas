// SPDX-License-Identifier: MPL-2.0

use super::PidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::{
            pid::task::{
                cgroup::CgroupFileOps, cmdline::CmdlineFileOps, comm::CommFileOps,
                environ::EnvironFileOps, exe::ExeSymOps, fd::FdDirOps, gid_map::GidMapFileOps,
                maps::MapsFileOps, mem::MemFileOps, mountinfo::MountInfoFileOps,
                mounts::MountsFileOps, ns::NsDirOps, oom_score_adj::OomScoreAdjFileOps,
                stat::StatFileOps, status::StatusFileOps, uid_map::UidMapFileOps,
            },
            template::{
                DirOps, ProcDir, ProcDirBuilder, ReaddirEntry, keyed_readdir_entries,
                lookup_child_from_table, populate_children_from_table,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
    process::{Process, posix_thread::AsPosixThread},
    thread::{AsThread, Thread, Tid},
};

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
mod ns;
mod oom_score_adj;
mod stat;
mod status;
mod uid_map;

/// Represents the inode at `/proc/[pid]/task`.
pub struct TaskDirOps(Arc<Process>);

impl TaskDirOps {
    pub fn new_inode(dir: &PidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let process_ref = dir.0.process_ref.clone();
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3316>
        ProcDirBuilder::new(Self(process_ref), mkmod!(a+rx))
            .parent(parent)
            .need_neg_child_revalidation()
            .build()
            .unwrap()
    }

    fn thread_entries(&self, dir: &ProcDir<Self>) -> Vec<(usize, String, Arc<dyn Inode>)> {
        let mut threads: Vec<_> = {
            let tasks = self.0.tasks().lock();
            tasks
                .as_slice()
                .iter()
                .map(|task| task.as_thread().unwrap().clone())
                .collect()
        };
        threads.sort_unstable_by_key(|thread| thread.as_posix_thread().unwrap().tid());

        threads
            .into_iter()
            .filter_map(|thread_ref| {
                usize::try_from(thread_ref.as_posix_thread().unwrap().tid())
                    .ok()
                    .map(|tid| {
                        let inode = TidDirOps::new_inode(
                            self.0.clone(),
                            thread_ref.clone(),
                            dir.this_weak().clone(),
                        );
                        (tid, tid.to_string(), inode)
                    })
            })
            .collect()
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]`.
#[derive(Clone)]
pub(super) struct TidDirOps {
    pub(super) process_ref: Arc<Process>,
    /// If `thread_ref` is `None`, this corresponds to a process-level `/proc/[pid]/*` file.
    /// Otherwise, this corresponds to a thread-level `/proc/[pid]/task/[tid]/*` file.
    pub(super) thread_ref: Option<Arc<Thread>>,
}

impl TidDirOps {
    pub fn new_inode(
        process_ref: Arc<Process>,
        thread_ref: Arc<Thread>,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        ProcDirBuilder::new(
            Self {
                process_ref,
                thread_ref: Some(thread_ref),
            },
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3796>
            mkmod!(a+rx),
        )
        .parent(parent)
        .need_revalidation()
        .build()
        .unwrap()
    }

    pub fn thread(&self) -> Arc<Thread> {
        self.thread_ref
            .clone()
            .unwrap_or_else(|| self.process_ref.main_thread())
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(
        &'static str,
        fn(&TidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>,
    )] = &[
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
        ("ns", NsDirOps::new_inode),
        ("oom_score_adj", OomScoreAdjFileOps::new_inode),
        ("stat", StatFileOps::new_inode),
        ("status", StatusFileOps::new_inode),
        ("uid_map", UidMapFileOps::new_inode),
        ("maps", MapsFileOps::new_inode),
        ("mounts", MountsFileOps::new_inode),
    ];
}

impl DirOps for TidDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        self.lookup_child(dir.this_weak().clone(), name)
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        self.populate_children(dir.this_weak().clone())
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        !Self::STATIC_ENTRIES
            .iter()
            .any(|(entry_name, _)| *entry_name == name)
    }
}

impl TidDirOps {
    pub(super) fn lookup_child(
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

    pub(super) fn populate_children(
        &self,
        this_ptr: Weak<dyn Inode>,
    ) -> Vec<(String, Arc<dyn Inode>)> {
        let mut children = Vec::new();
        populate_children_from_table(&mut children, Self::STATIC_ENTRIES, |f| {
            (f)(self, this_ptr.clone())
        });
        children
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<Tid>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid TID");
        };

        for task in self.0.tasks().lock().as_slice() {
            let thread_ref = task.as_thread().unwrap();
            if thread_ref.as_posix_thread().unwrap().tid() != tid {
                continue;
            }

            return Ok(TidDirOps::new_inode(
                self.0.clone(),
                thread_ref.clone(),
                dir.this_weak().clone(),
            ));
        }

        return_errno_with_message!(Errno::ENOENT, "the thread does not exist")
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        self.thread_entries(dir)
            .into_iter()
            .map(|(_, name, inode)| (name, inode))
            .collect()
    }

    fn entries_from_offset(&self, dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        keyed_readdir_entries(offset, 2, self.thread_entries(dir))
    }

    fn revalidate_pos_child(&self, name: &str, child: &dyn Inode) -> bool {
        let Ok(tid) = name.parse::<Tid>() else {
            return false;
        };
        let Some(child) = child.downcast_ref::<ProcDir<TidDirOps>>() else {
            return false;
        };
        let Some(thread_ref) = child.inner().thread_ref.as_ref() else {
            return false;
        };

        for task in self.0.tasks().lock().as_slice() {
            let current_thread = task.as_thread().unwrap();
            if current_thread.as_posix_thread().unwrap().tid() == tid {
                return Arc::ptr_eq(thread_ref, current_thread);
            }
        }

        false
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        let Ok(tid) = name.parse::<Tid>() else {
            return true;
        };

        self.0
            .tasks()
            .lock()
            .as_slice()
            .iter()
            .all(|task| task.as_posix_thread().unwrap().tid() != tid)
    }
}
