// SPDX-License-Identifier: MPL-2.0

//! `/proc/net`.

#![short_vis_path::add(procfs)]

use aster_util::printer::VmPrinter;

use super::{
    StaticEntry,
    template::{ReaddirEntry, listed_entries_from_table, visit_listed_entries},
};
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{ProcDir, ProcDirOps, ProcFile, ProcFileOps, lookup_child_from_table},
        vfs::inode::Inode,
    },
    prelude::*,
};

pub(in procfs) struct NetDirOps;

impl NetDirOps {
    pub(in procfs) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] =
        &[("dhcp", InodeType::File, DhcpFileOps::new_inode)];
}

impl ProcDirOps for NetDirOps {
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

/// `/proc/net/dhcp`: the DHCP leases held by the kernel's DHCP client.
///
/// This file is not part of Linux: Linux has no in-kernel DHCP client. One line
/// per DHCP-configured interface, `eth0 10.0.2.15/24 10.0.2.2 dns 10.0.2.3`, or
/// `eth0 pending` while the lease is outstanding.
struct DhcpFileOps;

impl DhcpFileOps {
    fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for DhcpFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);
        write!(printer, "{}", crate::net::dhcp_status())?;
        Ok(printer.bytes_written())
    }
}
