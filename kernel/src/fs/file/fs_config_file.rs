// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use super::{AccessMode, CreationFlags, FileLike, InodeMode};
use crate::{
    events::IoEvents,
    fs::{
        file::file_table::FdFlags,
        pseudofs::AnonInodeFs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::Path,
            registry::{FsCreationCtx, FsType},
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Represents a filesystem configuration context opened by `fsopen`.
///
/// The file stores configuration supplied through `fsconfig` until the
/// filesystem is created. Once creation succeeds, further configuration is
/// rejected unless it is an explicit reconfiguration request.
pub(crate) struct FsConfigFile {
    context: Mutex<FsConfigContext>,
    pseudo_path: Path,
}

/// Stores the mutable state of a filesystem configuration context.
struct FsConfigContext {
    fs_type: &'static dyn FsType,
    fs_flags: FsFlags,
    source: Option<String>,
    mode: Option<InodeMode>,
    fs: Option<Arc<dyn FileSystem>>,
    /// Accumulates unrecognized fsconfig key=value options as a comma-separated
    /// string, so that filesystem-specific mount options (e.g. `minixdf`) set
    /// via the new mount API are forwarded to `FsCreationCtx`.
    extra_options: String,
}

impl FsConfigFile {
    /// Creates a filesystem configuration file for a filesystem type.
    pub(crate) fn new(fs_type: &'static dyn FsType) -> Self {
        Self {
            context: Mutex::new(FsConfigContext {
                fs_type,
                fs_flags: FsFlags::empty(),
                source: None,
                mode: None,
                fs: None,
                extra_options: String::new(),
            }),
            pseudo_path: AnonInodeFs::new_path(|_| "anon_inode:[fscontext]".to_string()),
        }
    }

    /// Sets a boolean filesystem configuration option.
    pub(crate) fn set_flag(&self, key: &str) -> Result<()> {
        let mut context = self.context.lock();
        if context.fs.is_some() {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }
        match key {
            "noswap" => Ok(()),
            "ro" => {
                context.fs_flags |= FsFlags::RDONLY;
                Ok(())
            }
            _ => {
                if !context.extra_options.is_empty() {
                    context.extra_options.push(',');
                }
                context.extra_options.push_str(key);
                Ok(())
            }
        }
    }

    /// Sets a string filesystem configuration option.
    pub(crate) fn set_string(&self, key: &str, value: &str) -> Result<()> {
        let mut context = self.context.lock();
        if context.fs.is_some() {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }
        match key {
            "source" => {
                context.source = Some(value.to_string());
                Ok(())
            }
            "mode" => {
                context.mode = Some(parse_octal_mode(value)?);
                Ok(())
            }
            "nr_inodes" | "size" => Ok(()),
            _ => {
                if !context.extra_options.is_empty() {
                    context.extra_options.push(',');
                }
                context.extra_options.push_str(key);
                context.extra_options.push('=');
                context.extra_options.push_str(value);
                Ok(())
            }
        }
    }

    /// Creates the configured filesystem.
    pub(crate) fn create_fs(&self, ctx: &Context) -> Result<()> {
        let mut context = self.context.lock();
        if context.fs.is_some() {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }

        let args_cstr = if context.extra_options.is_empty() {
            None
        } else {
            Some(CString::new(context.extra_options.as_str()).map_err(|_| {
                Error::with_message(Errno::EINVAL, "mount options contain null byte")
            })?)
        };
        let args_ref = args_cstr.as_deref();
        let fs_creation_ctx =
            FsCreationCtx::new(context.source.as_deref(), context.fs_flags, args_ref, ctx);
        let fs = context.fs_type.create(&fs_creation_ctx)?;
        if let Some(mode) = context.mode {
            fs.root_inode().set_mode(mode)?;
        }
        context.fs = Some(fs);
        Ok(())
    }

    /// Reconfigures the created filesystem with the current flags.
    pub(crate) fn reconfigure_fs(&self, ctx: &Context) -> Result<()> {
        let context = self.context.lock();
        let fs = context
            .fs
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file system is not created"))?;

        fs.set_fs_flags(context.fs_flags, None, ctx)
    }

    /// Returns the created filesystem.
    pub(crate) fn created_fs(&self) -> Result<Arc<dyn FileSystem>> {
        self.context
            .lock()
            .fs
            .clone()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file system is not created"))
    }

    /// Returns the configured source.
    pub(crate) fn source(&self) -> Option<String> {
        self.context.lock().source.clone()
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

fn parse_octal_mode(value: &str) -> Result<InodeMode> {
    let mode = u16::from_str_radix(value, 8)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid octal mode"))?;
    if mode & !0o7777 != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid mode bits");
    }
    Ok(InodeMode::from_bits_truncate(mode))
}
