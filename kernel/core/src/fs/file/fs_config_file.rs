// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use super::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags};
use crate::{
    events::IoEvents,
    fs::{
        file::file_table::FdFlags,
        pseudofs::AnonInodeFs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{Mount, MountNamespace, Path, PerMountFlags},
            registry::{DynFsType, FsAndRoot, FsCreationCtx},
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Represents a filesystem configuration context.
///
/// The file stores configuration until the filesystem is created. Once creation
/// succeeds, further configuration is rejected unless it is an explicit
/// reconfiguration request.
///
/// The context has four states:
///
/// - `Configuring`: accepts parameters for filesystem creation.
/// - `AwaitingMount`: contains a created filesystem and accepts a request to
///   create one detached mount. A failed request leaves the context in this
///   state so that it can be retried.
/// - `Reconfiguring`: accepts parameters for reconfiguring the created
///   filesystem. A successful reconfiguration leaves the context in this state.
/// - `Failed`: indicates that filesystem creation or reconfiguration failed and
///   rejects further operations.
///
/// The state transitions are:
///
/// ```text
/// ┌─────────────┐  create_fs  ┌───────────────┐  create_detached_mount  ┌───────────────┐
/// │ Configuring │ ──────────> │ AwaitingMount │ ──────────────────────> │ Reconfiguring │
/// └──────┬──────┘             └───────────────┘                         └───────┬───────┘
///        │ create_fs fails                                 reconfigure_fs fails │
///        └──────────────────────────────┬───────────────────────────────────────┘
///                                       ▼
///                              ┌────────────────┐
///                              │     Failed     │
///                              └────────────────┘
/// ```
pub(crate) struct FsConfigFile {
    fs_type: &'static dyn DynFsType,
    config: Mutex<FsConfig>,
    common: FileCommon,
}

struct FsConfig {
    flags: FsFlags,
    source: Option<String>,
    /// Accumulates filesystem-specific mount options as a comma-separated string
    /// until filesystem creation or reconfiguration.
    extra_options: String,
    state: FsConfigState,
}

enum FsConfigState {
    Configuring,
    Failed,
    AwaitingMount(FsAndRoot),
    Reconfiguring(Arc<dyn FileSystem>),
}

impl FsConfigFile {
    /// Creates a filesystem configuration file for a filesystem type.
    pub(crate) fn new(fs_type: &'static dyn DynFsType) -> Self {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:[fscontext]".to_string());
        Self {
            fs_type,
            config: Mutex::new(FsConfig {
                flags: FsFlags::empty(),
                source: None,
                extra_options: String::new(),
                state: FsConfigState::Configuring,
            }),
            // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/fsopen.c#L97>.
            common: FileCommon::new(pseudo_path, AccessMode::O_RDWR, StatusFlags::empty()),
        }
    }

    /// Sets a flag-style filesystem configuration option.
    ///
    /// This is valid in the `Configuring` and `Reconfiguring` states.
    pub(crate) fn set_flag(&self, key: &str) -> Result<()> {
        let mut config = self.config.lock();
        match &config.state {
            FsConfigState::Configuring | FsConfigState::Reconfiguring(_) => match key {
                "ro" => {
                    config.flags |= FsFlags::RDONLY;
                    Ok(())
                }
                _ => {
                    // TODO: Validate filesystem-specific mount options when filesystem APIs
                    // expose their supported options.
                    append_option(&mut config.extra_options, key, None);
                    Ok(())
                }
            },
            FsConfigState::AwaitingMount(_) => {
                return_errno_with_message!(Errno::EBUSY, "the file system is awaiting fsmount");
            }
            FsConfigState::Failed => {
                return_errno_with_message!(Errno::EBUSY, "the file system configuration failed");
            }
        }
    }

    /// Sets a string filesystem configuration option.
    ///
    /// This is valid in the `Configuring` and `Reconfiguring` states. The
    /// `source` option may only be specified once in the `Configuring` state.
    pub(crate) fn set_string(&self, key: &str, value: &str) -> Result<()> {
        let mut config = self.config.lock();
        match &config.state {
            FsConfigState::Configuring => match key {
                "source" => {
                    if config.source.is_some() {
                        return_errno_with_message!(
                            Errno::EINVAL,
                            "the source is already specified"
                        );
                    }
                    config.source = Some(value.to_string());
                    Ok(())
                }
                _ => {
                    // TODO: Validate filesystem-specific mount options when filesystem APIs
                    // expose their supported options.
                    append_option(&mut config.extra_options, key, Some(value));
                    Ok(())
                }
            },
            FsConfigState::Reconfiguring(_) => {
                append_option(&mut config.extra_options, key, Some(value));
                Ok(())
            }
            FsConfigState::AwaitingMount(_) => {
                return_errno_with_message!(Errno::EBUSY, "the file system is awaiting fsmount");
            }
            FsConfigState::Failed => {
                return_errno_with_message!(Errno::EBUSY, "the file system configuration failed");
            }
        }
    }

    /// Creates the configured filesystem.
    ///
    /// This is only valid in the `Configuring` state. Success transitions to
    /// `AwaitingMount`, while failure transitions to `Failed`. It returns
    /// `EBUSY` in all other states.
    pub(crate) fn create_fs(&self, ctx: &Context) -> Result<()> {
        let mut config = self.config.lock();
        if !matches!(&config.state, FsConfigState::Configuring) {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }

        let result = (|| {
            let args = (!config.extra_options.is_empty()).then_some(config.extra_options.as_str());
            let mut fs_creation_ctx =
                FsCreationCtx::new(config.source.as_deref(), config.flags, args, ctx);
            let fs_and_root = self.fs_type.get_or_create(&mut fs_creation_ctx)?;
            Ok(fs_and_root)
        })();

        match result {
            Ok(fs_and_root) => {
                config.state = FsConfigState::AwaitingMount(fs_and_root);
                Ok(())
            }
            Err(err) => {
                config.state = FsConfigState::Failed;
                Err(err)
            }
        }
    }

    /// Creates a detached mount from the created filesystem.
    ///
    /// This is only valid in the `AwaitingMount` state. Success transitions to
    /// `Reconfiguring`, while failure leaves the context in `AwaitingMount` so
    /// that the operation can be retried.
    pub(crate) fn create_detached_mount(
        &self,
        flags: PerMountFlags,
        mnt_ns: Weak<MountNamespace>,
    ) -> Result<Arc<Mount>> {
        let mut config = self.config.lock();
        let fs_and_root = match &config.state {
            FsConfigState::AwaitingMount(fs_and_root) => fs_and_root.clone(),
            FsConfigState::Configuring => {
                return_errno_with_message!(Errno::EINVAL, "the file system is not created");
            }
            FsConfigState::Reconfiguring(_) => {
                return_errno_with_message!(
                    Errno::EBUSY,
                    "a detached mount has already been created"
                );
            }
            FsConfigState::Failed => {
                return_errno_with_message!(Errno::EBUSY, "the file system configuration failed");
            }
        };

        let fs = fs_and_root.fs().clone();
        let detached_mount =
            Mount::new_detached(fs_and_root, flags, mnt_ns, config.source.clone())?;
        config.source = None;
        config.flags = fs.flags();
        config.extra_options.clear();
        config.state = FsConfigState::Reconfiguring(fs);

        Ok(detached_mount)
    }

    /// Reconfigures the created filesystem with the current parameters.
    ///
    /// This is only valid in the `Reconfiguring` state. Success leaves the
    /// context in `Reconfiguring`, while failure transitions to `Failed`.
    pub(crate) fn reconfigure_fs(&self, ctx: &Context) -> Result<()> {
        let mut config = self.config.lock();
        let FsConfigState::Reconfiguring(fs) = &config.state else {
            return_errno_with_message!(Errno::EBUSY, "the file system is not reconfiguring");
        };
        let fs = fs.clone();

        let data = (!config.extra_options.is_empty()).then_some(config.extra_options.as_str());
        let result = fs.set_fs_flags(config.flags, data, ctx);
        if let Err(err) = result {
            config.state = FsConfigState::Failed;
            return Err(err);
        }

        // Discard parameters after a successful reconfiguration.
        // Reference:
        // <https://elixir.bootlin.com/linux/v7.0/source/fs/fs_context.c#L536>.
        config.flags = fs.flags();
        config.extra_options.clear();
        Ok(())
    }
}

impl Pollable for FsConfigFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for FsConfigFile {
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
            access_mode: self.common.access_mode(),
            status_flags: self.common.status_flags(),
            fd_flags,
        })
    }
}

/// Represents a detached mount.
pub(crate) struct DetachedMountFile {
    mount: Arc<Mount>,
    common: FileCommon,
}

impl DetachedMountFile {
    /// Creates a detached mount file.
    pub(crate) fn new(mount: Arc<Mount>) -> Self {
        let root_path = Path::new_fs_root(mount.clone());
        Self {
            mount,
            // Reference: <https://elixir.bootlin.com/linux/v7.0/source/fs/namespace.c#L4497>.
            common: FileCommon::new(root_path, AccessMode::O_RDONLY, StatusFlags::empty()),
        }
    }

    /// Returns the detached mount.
    pub(crate) fn mount(&self) -> Arc<Mount> {
        self.mount.clone()
    }
}

impl Pollable for DetachedMountFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for DetachedMountFile {
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
            access_mode: self.common.access_mode(),
            status_flags: self.common.status_flags(),
            fd_flags,
        })
    }
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
