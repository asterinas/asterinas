use core::sync::atomic::{AtomicUsize, Ordering};

use crate::events::Observer;
use crate::fs::utils::{DirEntryVecExt, FileSystem, FsFlags, Inode, SuperBlock, NAME_MAX};
use crate::prelude::*;
use crate::process::{process_table, process_table::PidEvent, Pid};

use self::pid::PidDirOps;
use self::self_::SelfSymOps;
use self::template::{DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps};

mod pid;
mod self_;
mod template;

/// Magic number.
const PROC_MAGIC: u64 = 0x9fa0;
/// Root Inode ID.
const PROC_ROOT_INO: usize = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

pub struct ProcFS {
    sb: RwLock<SuperBlock>,
    root: RwLock<Option<Arc<dyn Inode>>>,
    inode_allocator: AtomicUsize,
}

impl ProcFS {
    pub fn new() -> Arc<Self> {
        let procfs = {
            let sb = SuperBlock::new(PROC_MAGIC, BLOCK_SIZE, NAME_MAX);
            Arc::new(Self {
                sb: RwLock::new(sb),
                root: RwLock::new(None),
                inode_allocator: AtomicUsize::new(PROC_ROOT_INO),
            })
        };

        let root = RootDirOps::new_inode(&procfs);
        *procfs.root.write() = Some(root);
        procfs
    }

    pub(in crate::fs::procfs) fn alloc_id(&self) -> usize {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }
}

impl FileSystem for ProcFS {
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.read().as_ref().unwrap().clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.read().clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
    }
}

/// Represents the inode at `/proc`.
struct RootDirOps;

impl RootDirOps {
    pub fn new_inode(fs: &Arc<ProcFS>) -> Arc<dyn Inode> {
        let root_inode = ProcDirBuilder::new(Self).fs(fs.clone()).build().unwrap();
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
        let child = if name == "self" {
            SelfSymOps::new_inode(this_ptr.clone())
        } else if let Ok(pid) = name.parse::<Pid>() {
            let process_ref =
                process_table::get_process(&pid).ok_or_else(|| Error::new(Errno::ENOENT))?;
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
        cached_children.put_entry_if_not_found("self", || SelfSymOps::new_inode(this_ptr.clone()));

        let processes = process_table::get_all_processes();
        for process in processes {
            let pid = process.pid().to_string();
            cached_children.put_entry_if_not_found(&pid, || {
                PidDirOps::new_inode(process.clone(), this_ptr.clone())
            });
        }
    }
}
