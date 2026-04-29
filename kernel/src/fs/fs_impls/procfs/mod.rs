// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use template::{
    DirOps, ListedEntry, ProcDir, ReaddirEntry, StaticDirEntry, keyed_readdir_entries,
    listed_entries_from_table, lookup_child_from_table, sequential_readdir_entries,
    visit_readdir_entries,
};

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
    fs::{
        file::{InodeType, mkmod},
        procfs::{filesystems::FileSystemsFileOps, stat::StatFileOps},
        pseudofs::AnonDeviceId,
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::{Inode, RevalidationPolicy},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    process::{
        Pid,
        pid_table::{self, PidEntryType},
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

    fn create(&self, _fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
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
        ProcDir::new_root(Self, fs, PROC_ROOT_INO, sb, mkmod!(a+rx))
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [StaticDirEntry<fn(Weak<dyn Inode>) -> Arc<dyn Inode>>] = &[
        ("cmdline", InodeType::File, CmdLineFileOps::new_inode),
        ("cpuinfo", InodeType::File, CpuInfoFileOps::new_inode),
        (
            "filesystems",
            InodeType::File,
            FileSystemsFileOps::new_inode,
        ),
        ("loadavg", InodeType::File, LoadAvgFileOps::new_inode),
        ("meminfo", InodeType::File, MemInfoFileOps::new_inode),
        ("mounts", InodeType::SymLink, MountsSymOps::new_inode),
        ("self", InodeType::SymLink, SelfSymOps::new_inode),
        ("stat", InodeType::File, StatFileOps::new_inode),
        ("sys", InodeType::Dir, SysDirOps::new_inode),
        (
            "thread-self",
            InodeType::SymLink,
            ThreadSelfSymOps::new_inode,
        ),
        ("uptime", InodeType::File, UptimeFileOps::new_inode),
        ("version", InodeType::File, VersionFileOps::new_inode),
    ];
}

impl DirOps for RootDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Ok(pid) = name.parse::<Pid>() {
            let pid_entry = {
                let pid_table = pid_table::pid_table_mut();
                pid_table.get_entry(pid)
            };
            if let Some(pid_entry) = pid_entry
                && let Some(type_) = pid_entry.type_()
            {
                return Ok(match type_ {
                    PidEntryType::Process => {
                        PidDirOps::new_inode(pid_entry, this_dir.this_weak().clone())
                    }
                    PidEntryType::Thread => {
                        TidDirOps::new_inode(pid_entry, this_dir.this_weak().clone())
                    }
                });
            }
        }

        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, mut visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        const FIRST_PID_OFFSET: usize = 2 + RootDirOps::STATIC_ENTRIES.len();
        visit_readdir_entries(
            sequential_readdir_entries(offset, 2, listed_entries_from_table(Self::STATIC_ENTRIES)),
            &mut visit_fn,
        )?;

        // Collect PIDs before visiting entries, as `visit_fn` may copy data to user memory.
        let process_pids = {
            let pid_table = pid_table::pid_table_mut();
            pid_table
                .iter_processes()
                .filter_map(|process| usize::try_from(process.pid()).ok())
                .collect::<Vec<_>>()
        };

        visit_readdir_entries(
            keyed_readdir_entries(offset, FIRST_PID_OFFSET, process_pids, |process_pid| {
                ListedEntry::new(process_pid.to_string(), InodeType::Dir)
            }),
            visit_fn,
        )
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        if name.parse::<Pid>().is_err() {
            // For non-numeric names, the set of valid children does not change over time,
            // so we can rely on the cache without revalidating.
            return true;
        };

        if let Some(child) = child.downcast_ref::<ProcDir<PidDirOps>>() {
            // If the child's `PidEntry` still has the associated process, it will not be removed
            // from the `PidTable` and remains alive.
            matches!(
                child.inner().pid_entry().type_(),
                Some(PidEntryType::Process)
            )
        } else if let Some(child) = child.downcast_ref::<ProcDir<TidDirOps>>() {
            // If the child's `PidEntry` still has the associated thread, it will not be removed
            // from the `PidTable` and remains alive.
            matches!(
                child.inner().pid_entry().type_(),
                Some(PidEntryType::Thread)
            )
        } else {
            false
        }
    }

    fn revalidate_absent(&self, name: &str) -> bool {
        let Ok(pid) = name.parse::<Pid>() else {
            return true;
        };

        let pid_entry = {
            let pid_table = pid_table::pid_table_mut();
            pid_table.get_entry(pid)
        };

        pid_entry.is_none_or(|pid_entry| pid_entry.type_().is_none())
    }
}
