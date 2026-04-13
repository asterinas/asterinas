// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_util::slot_vec::SlotVec;
use ostd::sync::RwMutexUpgradeableGuard;
use template::{DirOps, ProcDir, lookup_child_from_table, populate_children_from_table};

use self::{
    cmdline::CmdLineFileOps,
    cpuinfo::CpuInfoFileOps,
    loadavg::LoadAvgFileOps,
    meminfo::MemInfoFileOps,
    mounts::MountsSymOps,
    pid::{PidDirOps, TidDirOps},
    self_::SelfSymOps,
    sys::SysDirOps,
    thread_self::ThreadSelfSymOps,
    uptime::UptimeFileOps,
    version::VersionFileOps,
};
use crate::{
    events::Observer,
    fs::{
        file::mkmod,
        procfs::{filesystems::FileSystemsFileOps, stat::StatFileOps},
        pseudofs::AnonDeviceId,
        utils::{DirEntryVecExt, NAME_MAX},
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
            registry::{FsProperties, FsType},
        },
    },
    prelude::*,
    process::{
        Pid,
        pid_table::{self, PidEvent},
        posix_thread::AsPosixThread,
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
    crate::fs::vfs::registry::register(&ProcFsType).unwrap();
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
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl ProcFs {
    pub(self) fn new() -> Arc<Self> {
        let anon_device_id = AnonDeviceId::acquire().expect("no device ID is available for procfs");
        let sb = SuperBlock::new(PROC_MAGIC, BLOCK_SIZE, NAME_MAX, anon_device_id.id());
        Arc::new_cyclic(|weak_fs| Self {
            _anon_device_id: anon_device_id,
            sb: sb.clone(),
            root: RootDirOps::new_inode(weak_fs.clone(), &sb),
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
    pub fn new_inode(fs: Weak<ProcFs>, sb: &SuperBlock) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/root.c#L368>
        let fs: Weak<dyn FileSystem> = fs;
        let root_inode = ProcDir::new_root(Self, fs, PROC_ROOT_INO, sb, mkmod!(a+rx));

        let weak_ptr = Arc::downgrade(&root_inode);
        pid_table::pid_table_mut().register_observer(weak_ptr);

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
    // Lock order: PID table -> cached entries
    //
    // Note that inverting the lock order is non-trivial because `Observer::on_events` will be
    // called with the PID table locked.

    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Ok(pid) = name.parse::<Pid>() {
            let pid_table = pid_table::pid_table_mut();

            if let Some(process_ref) = pid_table.get_process(pid) {
                let mut cached_children = dir.cached_children().write();
                return Ok(cached_children
                    .put_entry_if_not_found(name, move || {
                        PidDirOps::new_inode(process_ref, dir.this_weak().clone())
                    })
                    .clone());
            }

            if let Some(thread_ref) = pid_table.get_thread(pid) {
                let process_ref = thread_ref.as_posix_thread().unwrap().process();
                return Ok(TidDirOps::new_inode(
                    process_ref,
                    thread_ref,
                    dir.this_weak().clone(),
                ));
            }
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
        let pid_table = pid_table::pid_table_mut();
        let mut cached_children = dir.cached_children().write();

        for process_ref in pid_table.iter_processes() {
            let pid = process_ref.pid().to_string();
            cached_children.put_entry_if_not_found(&pid, move || {
                PidDirOps::new_inode(process_ref, dir.this_weak().clone())
            });
        }

        drop(pid_table);

        populate_children_from_table(&mut cached_children, Self::STATIC_ENTRIES, |f| {
            (f)(dir.this_weak().clone())
        });

        cached_children.downgrade()
    }
}
