// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "breakdown_counters")]
use self::breakdown_counters::BreakdownCountersFileOps;
use self::{
    cpuinfo::CpuInfoFileOps,
    loadavg::LoadAvgFileOps,
    meminfo::MemInfoFileOps,
    pid::PidDirOps,
    self_::SelfSymOps,
    sys::SysDirOps,
    template::{DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps},
    thread_self::ThreadSelfSymOps,
};
use crate::{
    events::Observer,
    fs::{
        procfs::filesystems::FileSystemsFileOps,
        registry::{FsProperties, FsType},
        utils::{DirEntryVecExt, FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX},
    },
    prelude::*,
    process::{
        process_table::{self, PidEvent},
        Pid,
    },
};

#[cfg(feature = "breakdown_counters")]
pub mod breakdown_counters;
mod cpuinfo;
mod filesystems;
mod loadavg;
mod meminfo;
mod pid;
mod self_;
mod sys;
mod template;
mod thread_self;

pub(super) fn init() {
    cpuinfo::init();
    let procfs_type = Arc::new(ProcFsType);
    super::registry::register(procfs_type).unwrap();
}

/// Magic number.
const PROC_MAGIC: u64 = 0x9fa0;
/// Root Inode ID.
const PROC_ROOT_INO: u64 = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

pub struct ProcFS {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
}

impl ProcFS {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_fs| Self {
            sb: SuperBlock::new(PROC_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: RootDirOps::new_inode(weak_fs.clone()),
            inode_allocator: AtomicU64::new(PROC_ROOT_INO + 1),
        })
    }

    pub(in crate::fs::procfs) fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }
}

impl FileSystem for ProcFS {
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

struct ProcFsType;

impl FsType for ProcFsType {
    fn name(&self) -> &'static str {
        "proc"
    }

    fn create(
        &self,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
        _ctx: &Context,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(ProcFS::new())
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysBranchNode>> {
        None
    }
}

/// Represents the inode at `/proc`.
struct RootDirOps;

impl RootDirOps {
    pub fn new_inode(fs: Weak<ProcFS>) -> Arc<dyn Inode> {
        let root_inode = ProcDirBuilder::new(Self)
            .fs(fs)
            .ino(PROC_ROOT_INO)
            .build()
            .unwrap();
        let weak_ptr = Arc::downgrade(&root_inode);
        process_table::register_observer(weak_ptr);
        root_inode
    }
}

impl Observer<PidEvent> for ProcDir<RootDirOps> {
    fn on_events(&self, events: &PidEvent) {
        let PidEvent::Exit(pid) = events;
        let mut cached_children = self.cached_children().write();
        cached_children.remove_entry_by_name(&pid.to_string());
    }
}

impl DirOps for RootDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        #[cfg(feature = "breakdown_counters")]
        if name == "breakdown-counters" {
            return Ok(BreakdownCountersFileOps::new_inode(this_ptr.clone()));
        }

        let child = if name == "self" {
            SelfSymOps::new_inode(this_ptr.clone())
        } else if name == "sys" {
            SysDirOps::new_inode(this_ptr.clone())
        } else if name == "thread-self" {
            ThreadSelfSymOps::new_inode(this_ptr.clone())
        } else if name == "filesystems" {
            FileSystemsFileOps::new_inode(this_ptr.clone())
        } else if name == "meminfo" {
            MemInfoFileOps::new_inode(this_ptr.clone())
        } else if name == "loadavg" {
            LoadAvgFileOps::new_inode(this_ptr.clone())
        } else if name == "cpuinfo" {
            CpuInfoFileOps::new_inode(this_ptr.clone())
        } else if let Ok(pid) = name.parse::<Pid>() {
            let process_ref =
                process_table::get_process(pid).ok_or_else(|| Error::new(Errno::ENOENT))?;
            PidDirOps::new_inode(process_ref, this_ptr.clone())
        } else {
            return_errno!(Errno::ENOENT);
        };
        Ok(child)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<RootDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();

        #[cfg(feature = "breakdown_counters")]
        cached_children.put_entry_if_not_found("breakdown-counters", || {
            BreakdownCountersFileOps::new_inode(this_ptr.clone())
        });

        cached_children.put_entry_if_not_found("self", || SelfSymOps::new_inode(this_ptr.clone()));
        cached_children.put_entry_if_not_found("thread-self", || {
            ThreadSelfSymOps::new_inode(this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("sys", || SysDirOps::new_inode(this_ptr.clone()));
        cached_children.put_entry_if_not_found("filesystems", || {
            FileSystemsFileOps::new_inode(this_ptr.clone())
        });
        cached_children
            .put_entry_if_not_found("meminfo", || MemInfoFileOps::new_inode(this_ptr.clone()));
        cached_children
            .put_entry_if_not_found("loadavg", || LoadAvgFileOps::new_inode(this_ptr.clone()));
        cached_children
            .put_entry_if_not_found("cpuinfo", || CpuInfoFileOps::new_inode(this_ptr.clone()));
        for process in process_table::process_table_mut().iter() {
            let pid = process.pid().to_string();
            cached_children.put_entry_if_not_found(&pid, || {
                PidDirOps::new_inode(process.clone(), this_ptr.clone())
            });
        }
    }
}
