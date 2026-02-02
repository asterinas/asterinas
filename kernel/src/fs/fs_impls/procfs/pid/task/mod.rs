// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;

use super::PidDirOps;
use crate::{
    events::Observer,
    fs::{
        procfs::{
            pid::task::{
                cgroup::CgroupFileOps, cmdline::CmdlineFileOps, comm::CommFileOps,
                environ::EnvironFileOps, exe::ExeSymOps, fd::FdDirOps, gid_map::GidMapFileOps,
                maps::MapsFileOps, mem::MemFileOps, mountinfo::MountInfoFileOps,
                oom_score_adj::OomScoreAdjFileOps, stat::StatFileOps, status::StatusFileOps,
                uid_map::UidMapFileOps,
            },
            template::{
                DirOps, ProcDir, ProcDirBuilder, lookup_child_from_table,
                populate_children_from_table,
            },
        },
        utils::{DirEntryVecExt, Inode, mkmod},
    },
    prelude::*,
    process::{Process, posix_thread::AsPosixThread, task_set::TidEvent},
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
        let task_dir_inode = ProcDirBuilder::new(Self(process_ref), mkmod!(a+rx))
            .parent(parent)
            .build()
            .unwrap();

        let weak_ptr = Arc::downgrade(&task_dir_inode);
        dir.0.process_ref.tasks().lock().register_observer(weak_ptr);

        task_dir_inode
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
        ("oom_score_adj", OomScoreAdjFileOps::new_inode),
        ("stat", StatFileOps::new_inode),
        ("status", StatusFileOps::new_inode),
        ("uid_map", UidMapFileOps::new_inode),
        ("maps", MapsFileOps::new_inode),
    ];
}

impl DirOps for TidDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let mut cached_children = dir.cached_children().write();
        self.lookup_child_locked(&mut cached_children, dir.this_weak().clone(), name)
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let mut cached_children = dir.cached_children().write();
        self.populate_children_locked(&mut cached_children, dir.this_weak().clone());
        cached_children.downgrade()
    }
}

impl TidDirOps {
    pub(super) fn lookup_child_locked(
        &self,
        cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
        this_ptr: Weak<dyn Inode>,
        name: &str,
    ) -> Result<Arc<dyn Inode>> {
        if let Some(child) =
            lookup_child_from_table(name, cached_children, Self::STATIC_ENTRIES, |f| {
                (f)(self, this_ptr)
            })
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    pub(super) fn populate_children_locked(
        &self,
        cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
        this_ptr: Weak<dyn Inode>,
    ) {
        populate_children_from_table(cached_children, Self::STATIC_ENTRIES, |f| {
            (f)(self, this_ptr.clone())
        });
    }
}

impl Observer<TidEvent> for ProcDir<TaskDirOps> {
    fn on_events(&self, events: &TidEvent) {
        let TidEvent::Exit(tid) = events;

        let mut cached_children = self.cached_children().write();
        cached_children.remove_entry_by_name(&tid.to_string());
    }
}

impl DirOps for TaskDirOps {
    // Lock order: task set -> cached entries
    //
    // Note that inverting the lock order is non-trivial because `Observer::on_events` will be
    // called with the task set locked.

    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<Tid>() else {
            return_errno_with_message!(Errno::ENOENT, "the name is not a valid TID");
        };

        for task in self.0.tasks().lock().as_slice() {
            let thread_ref = task.as_thread().unwrap();
            if thread_ref.as_posix_thread().unwrap().tid() != tid {
                continue;
            }

            let mut cached_children = dir.cached_children().write();
            return Ok(cached_children
                .put_entry_if_not_found(name, || {
                    TidDirOps::new_inode(
                        self.0.clone(),
                        thread_ref.clone(),
                        dir.this_weak().clone(),
                    )
                })
                .clone());
        }

        return_errno_with_message!(Errno::ENOENT, "the thread does not exist")
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let tasks = self.0.tasks().lock();
        let mut cached_dentries = dir.cached_children().write();

        for task in tasks.as_slice() {
            let thread_ref = task.as_thread().unwrap();
            cached_dentries.put_entry_if_not_found(
                &task.as_posix_thread().unwrap().tid().to_string(),
                || {
                    TidDirOps::new_inode(
                        self.0.clone(),
                        thread_ref.clone(),
                        dir.this_weak().clone(),
                    )
                },
            );
        }

        cached_dentries.downgrade()
    }
}
