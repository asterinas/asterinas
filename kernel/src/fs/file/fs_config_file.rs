// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use super::{AccessMode, CreationFlags, FileCommon, FileLike, InodeMode, StatusFlags};
use crate::{
    events::IoEvents,
    fs::{
        file::file_table::FdFlags,
        pseudofs::AnonInodeFs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{Mount, MountNamespace, Path, PerMountFlags},
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
pub struct FsConfigFile {
    fs_type: &'static dyn FsType,
    state: Mutex<FsConfigState>,
    common: FileCommon,
}

enum FsConfigState {
    Configuring(FsCreationConfig),
    Created(Option<CreatedFs>),
}

struct FsCreationConfig {
    flags: FsFlags,
    source: Option<String>,
    mode: Option<InodeMode>,
    /// Accumulates filesystem-specific mount options as a comma-separated
    /// string, so they can be forwarded to `FsCreationCtx`.
    extra_options: String,
}

struct CreatedFs {
    fs: Arc<dyn FileSystem>,
    flags: FsFlags,
    source: Option<String>,
}

impl FsConfigFile {
    /// Creates a filesystem configuration file for a filesystem type.
    pub fn new(fs_type: &'static dyn FsType) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[fscontext]".to_string());
        Self {
            fs_type,
            state: Mutex::new(FsConfigState::Configuring(FsCreationConfig {
                flags: FsFlags::empty(),
                source: None,
                mode: None,
                extra_options: String::new(),
            })),
            common: FileCommon::new(pseudo_path, StatusFlags::empty()),
        }
    }

    /// Sets a flag-style filesystem configuration option.
    ///
    /// This is only valid while the context is still configuring the filesystem.
    /// It returns `EBUSY` after the filesystem has been created.
    pub fn set_flag(&self, key: &str) -> Result<()> {
        let mut state = self.state.lock();
        let FsConfigState::Configuring(config) = &mut *state else {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        };

        match key {
            "ro" => {
                config.flags |= FsFlags::RDONLY;
                Ok(())
            }
            _ => {
                append_option(&mut config.extra_options, key, None);
                Ok(())
            }
        }
    }

    /// Sets a string filesystem configuration option.
    ///
    /// This is only valid while the context is still configuring the filesystem.
    /// It returns `EBUSY` after the filesystem has been created. The `source`
    /// option may only be specified once.
    pub fn set_string(&self, key: &str, value: &str) -> Result<()> {
        let mut state = self.state.lock();
        let FsConfigState::Configuring(config) = &mut *state else {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        };

        match key {
            "source" => {
                if config.source.is_some() {
                    return_errno_with_message!(Errno::EINVAL, "the source is already specified");
                }
                config.source = Some(value.to_string());
                Ok(())
            }
            "mode" => {
                config.mode = Some(parse_octal_mode(value)?);
                Ok(())
            }
            _ => {
                append_option(&mut config.extra_options, key, Some(value));
                Ok(())
            }
        }
    }

    /// Creates the configured filesystem.
    ///
    /// This consumes the current creation configuration and moves the context
    /// into the created state. It returns `EBUSY` if the filesystem has already
    /// been created.
    pub fn create_fs(&self, ctx: &Context) -> Result<()> {
        let mut state = self.state.lock();
        let FsConfigState::Configuring(config) = &mut *state else {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        };

        let args_cstr = if config.extra_options.is_empty() {
            None
        } else {
            Some(CString::new(config.extra_options.as_str()).map_err(|_| {
                Error::with_message(Errno::EINVAL, "mount options contain null byte")
            })?)
        };
        let args_ref = args_cstr.as_deref();
        let fs_creation_ctx =
            FsCreationCtx::new(config.source.as_deref(), config.flags, args_ref, ctx);
        let fs = self.fs_type.create(&fs_creation_ctx)?;
        if Arc::strong_count(&fs) > 1 {
            let extant_readonly = fs.flags().contains(FsFlags::RDONLY);
            let context_readonly = config.flags.contains(FsFlags::RDONLY);
            if extant_readonly != context_readonly {
                return_errno_with_message!(
                    Errno::EBUSY,
                    "the read-only flag of the extant filesystem does not match"
                );
            }
        } else {
            if let Some(mode) = config.mode {
                fs.root_inode().set_mode(mode)?;
            }
        }
        let flags = config.flags;
        let source = config.source.clone();
        *state = FsConfigState::Created(Some(CreatedFs { fs, flags, source }));
        Ok(())
    }

    /// Reconfigures the created filesystem with the current flags.
    ///
    /// This is only valid after the filesystem has been created and before it
    /// has been consumed by `create_detached_mount`. It returns `EINVAL` if the
    /// filesystem has not been created yet, and `EBUSY` if it has already been
    /// consumed.
    pub fn reconfigure_fs(&self, ctx: &Context) -> Result<()> {
        let state = self.state.lock();
        let (fs, flags) = match &*state {
            FsConfigState::Created(Some(created)) => (&created.fs, created.flags),
            FsConfigState::Created(None) => {
                return_errno_with_message!(
                    Errno::EBUSY,
                    "the file system has already been mounted"
                );
            }
            FsConfigState::Configuring(_) => {
                return_errno_with_message!(Errno::EINVAL, "the file system is not created");
            }
        };

        fs.set_fs_flags(flags, None, ctx)
    }

    /// Creates a detached mount from the created filesystem.
    ///
    /// This is only valid after `create_fs` has succeeded and before a
    /// detached mount has already been created from this file. On success, the
    /// filesystem is consumed and subsequent calls return `EBUSY`. On failure,
    /// the filesystem remains available for a later `fsmount`.
    pub fn create_detached_mount(
        &self,
        flags: PerMountFlags,
        mnt_ns: Weak<MountNamespace>,
    ) -> Result<Arc<Mount>> {
        let mut state = self.state.lock();
        let FsConfigState::Created(created) = &mut *state else {
            return_errno_with_message!(Errno::EINVAL, "the file system is not created");
        };
        let Some(created_fs) = created.as_ref() else {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been mounted");
        };

        let detached_mount = Mount::new_detached(
            created_fs.fs.clone(),
            flags,
            mnt_ns,
            created_fs.source.clone(),
        )?;
        *created = None;

        Ok(detached_mount)
    }
}

impl Pollable for FsConfigFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for FsConfigFile {
    fn access_mode(&self) -> AccessMode {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/fsopen.c#L97>.
        AccessMode::O_RDWR
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            access_mode: AccessMode,
            status_flags: StatusFlags,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.status_flags.bits() | self.access_mode as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo {
            access_mode: self.access_mode(),
            status_flags: self.status_flags(),
            fd_flags,
        })
    }
}

/// Represents a detached mount returned by `fsmount`.
pub struct DetachedMountFile {
    mount: Arc<Mount>,
    common: FileCommon,
}

impl DetachedMountFile {
    /// Creates a detached mount file.
    pub fn new(mount: Arc<Mount>) -> Self {
        let root_path = Path::new_fs_root(mount.clone());
        Self {
            mount,
            common: FileCommon::new(root_path, StatusFlags::empty()),
        }
    }

    /// Returns the detached mount.
    pub fn mount(&self) -> Arc<Mount> {
        self.mount.clone()
    }
}

impl Pollable for DetachedMountFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for DetachedMountFile {
    fn access_mode(&self) -> AccessMode {
        // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/namespace.c#L4497>.
        AccessMode::O_RDONLY
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            access_mode: AccessMode,
            status_flags: StatusFlags,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.status_flags.bits() | self.access_mode as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }

        Box::new(FdInfo {
            access_mode: self.access_mode(),
            status_flags: self.status_flags(),
            fd_flags,
        })
    }
}

fn parse_octal_mode(value: &str) -> Result<InodeMode> {
    const MAX_MODE_BITS: u16 = 0o7777;

    let mode = u16::from_str_radix(value, 8)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid octal mode"))?;
    if mode & !MAX_MODE_BITS != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid mode bits");
    }
    Ok(InodeMode::from_bits_truncate(mode))
}

fn append_option(options: &mut String, key: &str, value: Option<&str>) {
    if !options.is_empty() {
        options.push(',');
    }
    options.push_str(key);
    if let Some(value) = value {
        options.push('=');
        options.push_str(value);
    }
}
