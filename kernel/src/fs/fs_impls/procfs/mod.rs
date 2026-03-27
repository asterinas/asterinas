// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use template::{
    DirOps, ProcDir, ReaddirEntry, keyed_readdir_entries, lookup_child_from_table,
    populate_children_from_table, sequential_readdir_entries,
};

use self::{
    cmdline::CmdLineFileOps, cpuinfo::CpuInfoFileOps, loadavg::LoadAvgFileOps,
    meminfo::MemInfoFileOps, mounts::MountsSymOps, pid::PidDirOps, self_::SelfSymOps,
    sys::SysDirOps, thread_self::ThreadSelfSymOps, uptime::UptimeFileOps, version::VersionFileOps,
};
use crate::{
    fs::{
        file::mkmod,
        procfs::{filesystems::FileSystemsFileOps, stat::StatFileOps},
        pseudofs::AnonDeviceId,
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
            registry::{FsProperties, FsType},
        },
    },
    prelude::*,
    process::{Pid, Process, process_table},
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
        ProcDir::new_root(Self, fs, PROC_ROOT_INO, sb, mkmod!(a+rx), true)
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

    fn contains_static_entry(name: &str) -> bool {
        Self::STATIC_ENTRIES
            .iter()
            .any(|(entry_name, _)| *entry_name == name)
    }

    fn get_process(pid: Pid) -> Option<Arc<Process>> {
        let process_table = process_table::process_table_mut();
        process_table.get(pid).cloned()
    }

    fn static_entries(dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        Self::STATIC_ENTRIES
            .iter()
            .map(|(name, constructor)| {
                ((*name).to_string(), (*constructor)(dir.this_weak().clone()))
            })
            .collect()
    }

    fn process_entries(dir: &ProcDir<Self>) -> Vec<(usize, String, Arc<dyn Inode>)> {
        let process_table = process_table::process_table_mut();
        process_table
            .iter()
            .filter_map(|process_ref| {
                usize::try_from(process_ref.pid()).ok().map(|pid| {
                    let inode = PidDirOps::new_inode(process_ref.clone(), dir.this_weak().clone());
                    (pid, pid.to_string(), inode)
                })
            })
            .collect()
    }
}

impl DirOps for RootDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Ok(pid) = name.parse::<Pid>()
            && let Some(process_ref) = Self::get_process(pid)
        {
            return Ok(PidDirOps::new_inode(process_ref, dir.this_weak().clone()));
        }

        if let Some(child) =
            lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| (f)(dir.this_weak().clone()))
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        let mut children = Self::process_entries(dir)
            .into_iter()
            .map(|(_, name, inode)| (name, inode))
            .collect();
        populate_children_from_table(&mut children, Self::STATIC_ENTRIES, |f| {
            (f)(dir.this_weak().clone())
        });
        children
    }

    fn entries_from_offset(&self, dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        const FIRST_PID_OFFSET: usize = 2 + RootDirOps::STATIC_ENTRIES.len();
        let mut entries = sequential_readdir_entries(offset, 2, Self::static_entries(dir));
        entries.extend(keyed_readdir_entries(
            offset,
            FIRST_PID_OFFSET,
            Self::process_entries(dir),
        ));
        entries
    }

    fn revalidate_pos_child(&self, name: &str, child: &dyn Inode) -> bool {
        let Ok(pid) = name.parse::<Pid>() else {
            return Self::contains_static_entry(name);
        };
        let Some(child) = child.downcast_ref::<ProcDir<PidDirOps>>() else {
            return false;
        };
        let Some(process_ref) = Self::get_process(pid) else {
            return false;
        };
        Arc::ptr_eq(&process_ref, child.inner().process_ref())
    }

    fn revalidate_neg_child(&self, name: &str) -> bool {
        if Self::contains_static_entry(name) {
            return false;
        }

        let Ok(pid) = name.parse::<Pid>() else {
            return true;
        };
        Self::get_process(pid).is_none()
    }
}
