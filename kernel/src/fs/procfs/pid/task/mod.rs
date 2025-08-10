// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_util::slot_vec::SlotVec;

use crate::{
    fs::{
        procfs::{
            pid::{
                stat::StatFileOps,
                task::{
                    cmdline::CmdlineFileOps, comm::CommFileOps, environ::EnvironFileOps,
                    exe::ExeSymOps, fd::FdDirOps, gid_map::GidMapFileOps, mem::MemFileOps,
                    oom_score_adj::OomScoreAdjFileOps, status::StatusFileOps,
                    uid_map::UidMapFileOps,
                },
            },
            template::{DirOps, ProcDir, ProcDirBuilder},
        },
        utils::{mkmod, DirEntryVecExt, Inode},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{AsThread, Thread},
    Process,
};

mod cmdline;
mod comm;
mod environ;
mod exe;
mod fd;
mod gid_map;
mod mem;
mod oom_score_adj;
mod status;
mod uid_map;

/// Represents the inode at `/proc/[pid]/task`.
pub struct TaskDirOps(Arc<Process>);

impl TaskDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3316>
        ProcDirBuilder::new(Self(process_ref), mkmod!(a+rx))
            .parent(parent)
            .build()
            .unwrap()
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]`.
#[derive(Clone)]
pub(super) struct TidDirOps {
    pub(super) process_ref: Arc<Process>,
    pub(super) thread_ref: Arc<Thread>,
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
                thread_ref,
            },
            // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3796>
            mkmod!(a+rx),
        )
        .parent(parent)
        .build()
        .unwrap()
    }
}

impl DirOps for TidDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "cmdline" => CmdlineFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "comm" => CommFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "environ" => EnvironFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "exe" => ExeSymOps::new_inode(self.process_ref.clone(), this_ptr),
            "fd" => FdDirOps::new_inode(self.thread_ref.clone(), this_ptr),
            "gid_map" => GidMapFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "mem" => MemFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "oom_score_adj" => OomScoreAdjFileOps::new_inode(self.process_ref.clone(), this_ptr),
            "stat" => StatFileOps::new_inode(
                self.process_ref.clone(),
                self.thread_ref.clone(),
                false,
                this_ptr,
            ),
            "status" => StatusFileOps::new_inode(
                self.process_ref.clone(),
                self.thread_ref.clone(),
                this_ptr,
            ),
            "uid_map" => UidMapFileOps::new_inode(self.process_ref.clone(), this_ptr),
            _ => return_errno!(Errno::ENOENT),
        };
        Ok(inode)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<TidDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        self.populate_children_inner(&mut cached_children, this_ptr);
    }
}

impl TidDirOps {
    pub(super) fn populate_children_inner(
        &self,
        cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
        this_ptr: Weak<dyn Inode>,
    ) {
        cached_children.put_entry_if_not_found("cmdline", || {
            CmdlineFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("comm", || {
            CommFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("environ", || {
            EnvironFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("exe", || {
            ExeSymOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("fd", || {
            FdDirOps::new_inode(self.thread_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("gid_map", || {
            GidMapFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("mem", || {
            MemFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("oom_score_adj", || {
            OomScoreAdjFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("stat", || {
            StatFileOps::new_inode(
                self.process_ref.clone(),
                self.thread_ref.clone(),
                false,
                this_ptr.clone(),
            )
        });
        cached_children.put_entry_if_not_found("status", || {
            StatusFileOps::new_inode(
                self.process_ref.clone(),
                self.thread_ref.clone(),
                this_ptr.clone(),
            )
        });
        cached_children.put_entry_if_not_found("uid_map", || {
            UidMapFileOps::new_inode(self.process_ref.clone(), this_ptr.clone())
        });
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<u32>() else {
            return_errno_with_message!(Errno::ENOENT, "Can not parse name to u32 type");
        };

        for task in self.0.tasks().lock().as_slice() {
            let thread = task.as_thread().unwrap();
            if thread.as_posix_thread().unwrap().tid() != tid {
                continue;
            }
            return Ok(TidDirOps::new_inode(
                self.0.clone(),
                thread.clone(),
                this_ptr,
            ));
        }
        return_errno_with_message!(Errno::ENOENT, "No such thread")
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<TaskDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        for task in self.0.tasks().lock().as_slice() {
            let thread = task.as_thread().unwrap();
            cached_children.put_entry_if_not_found(
                &format!("{}", task.as_posix_thread().unwrap().tid()),
                || TidDirOps::new_inode(self.0.clone(), thread.clone(), this_ptr.clone()),
            );
        }
    }
}
