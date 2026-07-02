// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use super::{AccessMode, CreationFlags, FileLike};
use crate::{
    events::IoEvents,
    fs::{
        file::file_table::FdFlags,
        pseudofs::AnonInodeFs,
        vfs::{path::Path, registry::FsType},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Represents a filesystem configuration context opened by `fsopen`.
///
/// The file stores configuration supplied through `fsconfig` until the
/// filesystem is created. Once creation succeeds, further configuration is
/// rejected unless it is an explicit reconfiguration request.
pub struct FsConfigFile {
    pseudo_path: Path,
}

impl FsConfigFile {
    /// Creates a filesystem configuration file for a filesystem type.
    pub fn new(_fs_type: &'static dyn FsType) -> Self {
        Self {
            pseudo_path: AnonInodeFs::new_path(|_| "anon_inode:[fscontext]".to_string()),
        }
    }
}

impl Pollable for FsConfigFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for FsConfigFile {
    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        Box::new(MountApiFdInfo {
            access_mode: self.access_mode(),
            fd_flags,
        })
    }
}

struct MountApiFdInfo {
    access_mode: AccessMode,
    fd_flags: FdFlags,
}

impl Display for MountApiFdInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut flags = self.access_mode as u32;
        if self.fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }

        writeln!(f, "pos:\t{}", 0)?;
        writeln!(f, "flags:\t0{:o}", flags)?;
        writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
        writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
    }
}
