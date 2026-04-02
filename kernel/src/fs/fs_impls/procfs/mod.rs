// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use template::{
    DirOps, ProcDir, ReaddirEntry, child_names_from_table, keyed_readdir_entries,
    lookup_child_from_table, sequential_readdir_entries,
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
            inode::{Inode, RevalidateResult},
            registry::{FsProperties, FsType},
        },
    },
    prelude::*,
    process::{
        Pid,
        pid_table::{self, PidEntry},
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

    fn get_pid_entry(pid: Pid) -> Option<Arc<PidEntry>> {
        let pid_table = pid_table::pid_table_mut();
        pid_table
            .get_entry(pid)
            .filter(|pid_entry| pid_entry.process().is_some())
    }

    fn process_entries() -> Vec<(usize, String)> {
        let pid_table = pid_table::pid_table_mut();
        pid_table
            .iter_process_entries()
            .filter_map(|pid_entry| {
                pid_entry.process().and_then(|process_ref| {
                    usize::try_from(process_ref.pid())
                        .ok()
                        .map(|process_pid| (process_pid, process_pid.to_string()))
                })
            })
            .collect()
    }
}

impl DirOps for RootDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Ok(pid) = name.parse::<Pid>()
            && let Some(pid_entry) = Self::get_pid_entry(pid)
        {
            return Ok(PidDirOps::new_inode(pid_entry, dir.this_weak().clone()));
        }

        if let Some(child) =
            lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| (f)(dir.this_weak().clone()))
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn child_names(&self, _dir: &ProcDir<Self>) -> Vec<String> {
        let mut children = Self::process_entries()
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>();
        children.extend(child_names_from_table(Self::STATIC_ENTRIES));
        children
    }

    fn entries_from_offset(&self, _dir: &ProcDir<Self>, offset: usize) -> Vec<ReaddirEntry> {
        const FIRST_PID_OFFSET: usize = 2 + RootDirOps::STATIC_ENTRIES.len();
        let mut entries =
            sequential_readdir_entries(offset, 2, child_names_from_table(Self::STATIC_ENTRIES));
        entries.extend(keyed_readdir_entries(
            offset,
            FIRST_PID_OFFSET,
            Self::process_entries(),
        ));
        entries
    }

    fn revalidate_pos_child(&self, name: &str, child: &dyn Inode) -> RevalidateResult {
        let Ok(pid) = name.parse::<Pid>() else {
            return RevalidateResult::Invalid;
        };
        let Some(child) = child.downcast_ref::<ProcDir<PidDirOps>>() else {
            return RevalidateResult::Invalid;
        };
        let Some(pid_entry) = Self::get_pid_entry(pid) else {
            return RevalidateResult::Invalid;
        };
        if Arc::ptr_eq(&pid_entry, child.inner().pid_entry()) {
            RevalidateResult::Valid
        } else {
            RevalidateResult::Invalid
        }
    }

    fn revalidate_neg_child(&self, name: &str) -> RevalidateResult {
        if Self::STATIC_ENTRIES
            .iter()
            .any(|(entry_name, _)| *entry_name == name)
        {
            return RevalidateResult::Valid;
        }

        let Ok(pid) = name.parse::<Pid>() else {
            return RevalidateResult::Valid;
        };
        if Self::get_pid_entry(pid).is_none() {
            RevalidateResult::Valid
        } else {
            RevalidateResult::Invalid
        }
    }
}
