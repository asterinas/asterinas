// SPDX-License-Identifier: MPL-2.0

//! The `/proc/sys/fs/mqueue` directory.

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            ProcDir, StaticEntry,
            template::{
                ProcDirOps, ReaddirEntry, listed_entries_from_table, lookup_child_from_table,
                visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

mod sysctl;

use sysctl::MqueueSysctl;

/// Represents the directory at `/proc/sys/fs/mqueue`.
pub(super) struct MqueueDirOps;

impl MqueueDirOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/ipc/mqueue.c#L2033>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("msg_default", InodeType::File, new_msg_default),
        ("msg_max", InodeType::File, new_msg_max),
        ("msgsize_default", InodeType::File, new_msgsize_default),
        ("msgsize_max", InodeType::File, new_msgsize_max),
        ("queues_max", InodeType::File, new_queues_max),
    ];
}

impl ProcDirOps for MqueueDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}

fn new_msg_default(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
    MqueueSysctl::MsgDefault.new_inode(parent)
}

fn new_msg_max(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
    MqueueSysctl::MsgMax.new_inode(parent)
}

fn new_msgsize_default(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
    MqueueSysctl::MsgsizeDefault.new_inode(parent)
}

fn new_msgsize_max(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
    MqueueSysctl::MsgsizeMax.new_inode(parent)
}

fn new_queues_max(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
    MqueueSysctl::QueuesMax.new_inode(parent)
}
