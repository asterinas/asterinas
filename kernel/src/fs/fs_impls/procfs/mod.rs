// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;
use template::{lookup_child_from_table, populate_children_from_table};

use self::{
    cmdline::CmdLineFileOps,
    cpuinfo::CpuInfoFileOps,
    loadavg::LoadAvgFileOps,
    meminfo::MemInfoFileOps,
    mounts::MountsSymOps,
    pid::PidDirOps,
    self_::SelfSymOps,
    sys::SysDirOps,
    template::{DirOps, ProcDir, ProcDirBuilder, ProcSymBuilder, SymOps},
    thread_self::ThreadSelfSymOps,
    uptime::UptimeFileOps,
    version::VersionFileOps,
};
use crate::{
    events::Observer,
    fs::{
        procfs::{filesystems::FileSystemsFileOps, stat::StatFileOps},
        registry::{FsProperties, FsType},
        utils::{
            DirEntryVecExt, FileSystem, FsEventSubscriberStats, FsFlags, Inode, NAME_MAX,
            SuperBlock, mkmod,
        },
    },
    prelude::*,
    process::{
        Pid,
        process_table::{self, PidEvent},
    },
};

mod cmdline;
mod cpuinfo;
mod filesystems;
mod loadavg;
mod meminfo;
mod mounts;
mod pid;
mod self_;
mod stat;
mod sys;
mod template;
mod thread_self;
mod uptime;
mod version;

pub(super) fn init() {
    super::registry::register(&ProcFsType).unwrap();
}

pub(super) fn init_on_each_cpu() {
    cpuinfo::init_on_each_cpu();
}

/// Magic number.
const PROC_MAGIC: u64 = 0x9fa0;
/// Root Inode ID.
const PROC_ROOT_INO: u64 = 1;
/// Block size.
const BLOCK_SIZE: usize = 1024;

struct ProcFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl ProcFs {
    pub(self) fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_fs| Self {
            sb: SuperBlock::new(PROC_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: RootDirOps::new_inode(weak_fs.clone()),
            inode_allocator: AtomicU64::new(PROC_ROOT_INO + 1),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }

    pub(self) fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::Relaxed)
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &'static str {
        "proc"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

struct ProcFsType;

impl FsType for ProcFsType {
    fn name(&self) -> &'static str {
        "proc"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(ProcFs::new())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

/// Represents the inode at `/proc`.
struct RootDirOps;

impl RootDirOps {
    pub fn new_inode(fs: Weak<ProcFs>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/root.c#L368>
        let root_inode = ProcDirBuilder::new(Self, mkmod!(a+rx))
            .fs(fs)
            .ino(PROC_ROOT_INO)
            .build()
            .unwrap();

        let weak_ptr = Arc::downgrade(&root_inode);
        process_table::register_observer(weak_ptr);

        root_inode
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(&'static str, fn(Weak<dyn Inode>) -> Arc<dyn Inode>)] = &[
        ("cmdline", CmdLineFileOps::new_inode),
        ("cpuinfo", CpuInfoFileOps::new_inode),
        ("filesystems", FileSystemsFileOps::new_inode),
        ("loadavg", LoadAvgFileOps::new_inode),
        ("meminfo", MemInfoFileOps::new_inode),
        ("mounts", MountsSymOps::new_inode),
        ("self", SelfSymOps::new_inode),
        ("stat", StatFileOps::new_inode),
        ("sys", SysDirOps::new_inode),
        ("thread-self", ThreadSelfSymOps::new_inode),
        ("uptime", UptimeFileOps::new_inode),
        ("version", VersionFileOps::new_inode),
    ];
}

impl Observer<PidEvent> for ProcDir<RootDirOps> {
    fn on_events(&self, events: &PidEvent) {
        let PidEvent::Exit(pid) = events;

        let mut cached_children = self.cached_children().write();
        cached_children.remove_entry_by_name(&pid.to_string());
    }
}

impl DirOps for RootDirOps {
    // Lock order: process table -> cached entries
    //
    // Note that inverting the lock order is non-trivial because `Observer::on_events` will be
    // called with the process table locked.

    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Ok(pid) = name.parse::<Pid>()
            && let process_table_mut = process_table::process_table_mut()
            && let Some(process_ref) = process_table_mut.get(pid)
        {
            let mut cached_children = dir.cached_children().write();
            return Ok(cached_children
                .put_entry_if_not_found(name, || {
                    PidDirOps::new_inode(process_ref.clone(), dir.this_weak().clone())
                })
                .clone());
        }

        let mut cached_children = dir.cached_children().write();

        if let Some(child) =
            lookup_child_from_table(name, &mut cached_children, Self::STATIC_ENTRIES, |f| {
                (f)(dir.this_weak().clone())
            })
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn populate_children<'a>(
        &self,
        dir: &'a ProcDir<Self>,
    ) -> RwMutexUpgradeableGuard<'a, SlotVec<(String, Arc<dyn Inode>)>> {
        let process_table_mut = process_table::process_table_mut();
        let mut cached_children = dir.cached_children().write();

        for process_ref in process_table_mut.iter() {
            let pid = process_ref.pid().to_string();
            cached_children.put_entry_if_not_found(&pid, || {
                PidDirOps::new_inode(process_ref.clone(), dir.this_weak().clone())
            });
        }

        drop(process_table_mut);

        populate_children_from_table(&mut cached_children, Self::STATIC_ENTRIES, |f| {
            (f)(dir.this_weak().clone())
        });

        cached_children.downgrade()
    }
}
