// SPDX-License-Identifier: MPL-2.0

use core::fmt::Display;

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    events::IoEvents,
    fs::{
        file::{
            AccessMode, CreationFlags, FileLike, InodeMode,
            file_table::{FdFlags, RawFileDesc, get_file_fast},
        },
        pseudofs::AnonInodeFs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{EmptyPathStr, FsPath, Mount, Path, PerMountFlags},
            registry::{FsCreationCtx, FsType, look_up},
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub fn sys_fsopen(fs_name_addr: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = FsOpenFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsopen flags"))?;
    let fs_name = ctx
        .user_space()
        .read_cstring(fs_name_addr, MAX_FILENAME_LEN)?;
    let fs_name = fs_name
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid file system name"))?;
    let fs_type = look_up(fs_name).ok_or_else(|| {
        Error::with_message(
            Errno::ENODEV,
            "the filesystem is not configured in the kernel",
        )
    })?;

    let file = Arc::new(FsContextFile::new(fs_type)) as Arc<dyn FileLike>;
    let fd_flags = if flags.contains(FsOpenFlags::FSOPEN_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    let fd = ctx
        .thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .insert(file, fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
}

pub fn sys_fsconfig(
    fs_fd: RawFileDesc,
    cmd: u32,
    key_addr: Vaddr,
    value_addr: Vaddr,
    _aux: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fs_fd.try_into()?);
    let fs_context = file
        .downcast_ref::<FsContextFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;

    match cmd {
        FSCONFIG_SET_FLAG => {
            let key = read_user_str(key_addr, ctx)?;
            fs_context.set_flag(key.to_str()?)?;
        }
        FSCONFIG_SET_STRING => {
            let key = read_user_str(key_addr, ctx)?;
            let value = read_user_str(value_addr, ctx)?;
            fs_context.set_string(key.to_str()?, value.to_str()?)?;
        }
        FSCONFIG_CMD_CREATE => fs_context.create_fs(ctx)?,
        FSCONFIG_CMD_RECONFIGURE => fs_context.reconfigure_fs(ctx)?,
        _ => return_errno_with_message!(Errno::EINVAL, "unsupported fsconfig command"),
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_fsmount(
    fs_fd: RawFileDesc,
    flags: u32,
    mount_attrs: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = FsMountFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown fsmount flags"))?;
    let per_mount_flags = mount_attrs_to_per_mount_flags(mount_attrs)?;
    let (fs, source) = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fs_fd.try_into()?);
        let fs_context = file
            .downcast_ref::<FsContextFile>()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a fs context"))?;
        (fs_context.created_fs()?, fs_context.source())
    };

    let current_ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let current_mnt_ns = current_ns_proxy.unwrap().mnt_ns();
    let detached_mount = Mount::new_detached(
        fs.clone(),
        per_mount_flags,
        Arc::downgrade(current_mnt_ns),
        source,
    )?;
    let file = Arc::new(DetachedMountFile::new(detached_mount)) as Arc<dyn FileLike>;
    let fd_flags = if flags.contains(FsMountFlags::FSMOUNT_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    let fd = ctx
        .thread_local
        .borrow_file_table()
        .unwrap()
        .write()
        .insert(file, fd_flags);
    Ok(SyscallReturn::Return(fd.into()))
}

pub fn sys_move_mount(
    from_dfd: RawFileDesc,
    from_path_addr: Vaddr,
    to_dfd: RawFileDesc,
    to_path_addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = MoveMountFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown move_mount flags"))?;
    if flags != MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH {
        return_errno_with_message!(Errno::EINVAL, "unsupported move_mount flags");
    }

    let from_path = read_user_str(from_path_addr, ctx)?;
    if !from_path.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "only empty from_path is supported");
    }

    let detached_mount = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, from_dfd.try_into()?);
        let mount_file = file.downcast_ref::<DetachedMountFile>().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the file is not a detached mount")
        })?;
        mount_file.mount()
    };

    let to_path = read_user_str(to_path_addr, ctx)?;
    let to_path = to_path
        .to_str()
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid target path"))?;
    let fs_path = FsPath::from_fd_at(to_dfd, to_path, EmptyPathStr::Reject)?;
    let target_path = ctx
        .thread_local
        .borrow_fs()
        .resolver()
        .read()
        .lookup(&fs_path)?
        .get_top_path();

    target_path.attach_detached_mount(&detached_mount, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn read_user_str(addr: Vaddr, ctx: &Context) -> Result<CString> {
    ctx.user_space().read_cstring(addr, MAX_FILENAME_LEN)
}

struct FsContextFile {
    state: Mutex<FsContextState>,
    pseudo_path: Path,
}

struct FsContextState {
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

impl FsContextFile {
    fn new(fs_type: &'static dyn FsType) -> Self {
        Self {
            state: Mutex::new(FsContextState {
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

    fn set_flag(&self, key: &str) -> Result<()> {
        let mut state = self.state.lock();
        match key {
            "noswap" => Ok(()),
            "ro" => {
                drop(state);
                self.set_fs_flags(FsFlags::RDONLY)
            }
            _ => {
                if !state.extra_options.is_empty() {
                    state.extra_options.push(',');
                }
                state.extra_options.push_str(key);
                Ok(())
            }
        }
    }

    fn set_string(&self, key: &str, value: &str) -> Result<()> {
        let mut state = self.state.lock();
        if state.fs.is_some() {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }
        match key {
            "source" => {
                state.source = Some(value.to_string());
                Ok(())
            }
            "mode" => {
                state.mode = Some(parse_octal_mode(value)?);
                Ok(())
            }
            "nr_inodes" | "size" => Ok(()),
            _ => {
                if !state.extra_options.is_empty() {
                    state.extra_options.push(',');
                }
                state.extra_options.push_str(key);
                state.extra_options.push('=');
                state.extra_options.push_str(value);
                Ok(())
            }
        }
    }

    fn create_fs(&self, ctx: &Context) -> Result<()> {
        let mut state = self.state.lock();
        if state.fs.is_some() {
            return_errno_with_message!(Errno::EBUSY, "the file system has already been created");
        }

        let args_cstr = if state.extra_options.is_empty() {
            None
        } else {
            Some(CString::new(state.extra_options.as_str()).map_err(|_| {
                Error::with_message(Errno::EINVAL, "mount options contain null byte")
            })?)
        };
        let args_ref = args_cstr.as_deref();
        let fs_creation_ctx =
            FsCreationCtx::new(state.source.as_deref(), state.fs_flags, args_ref, ctx);
        let fs = state.fs_type.create(&fs_creation_ctx)?;
        if let Some(mode) = state.mode {
            fs.root_inode().set_mode(mode)?;
        }
        state.fs = Some(fs);
        Ok(())
    }

    fn reconfigure_fs(&self, ctx: &Context) -> Result<()> {
        let state = self.state.lock();
        let fs = state
            .fs
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file system is not created"))?;

        fs.set_fs_flags(state.fs_flags, None, ctx)
    }

    fn set_fs_flags(&self, flags: FsFlags) -> Result<()> {
        let mut state = self.state.lock();
        state.fs_flags |= flags;
        Ok(())
    }

    fn created_fs(&self) -> Result<Arc<dyn FileSystem>> {
        self.state
            .lock()
            .fs
            .clone()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file system is not created"))
    }

    fn source(&self) -> Option<String> {
        self.state.lock().source.clone()
    }
}

impl Pollable for FsContextFile {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileLike for FsContextFile {
    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        Box::new(MountApiFdInfo { fd_flags })
    }
}

struct DetachedMountFile {
    mount: Arc<Mount>,
    root_path: Path,
}

impl DetachedMountFile {
    fn new(mount: Arc<Mount>) -> Self {
        let root_path = Path::new_fs_root(mount.clone());
        Self { mount, root_path }
    }

    fn mount(&self) -> Arc<Mount> {
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
        AccessMode::O_RDONLY
    }

    fn path(&self) -> &Path {
        &self.root_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        Box::new(MountApiFdInfo { fd_flags })
    }
}

struct MountApiFdInfo {
    fd_flags: FdFlags,
}

impl Display for MountApiFdInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut flags = AccessMode::O_RDONLY as u32;
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

fn mount_attrs_to_per_mount_flags(attrs: u32) -> Result<PerMountFlags> {
    let supported = MOUNT_ATTR_RDONLY
        | MOUNT_ATTR_NOSUID
        | MOUNT_ATTR_NODEV
        | MOUNT_ATTR_NOEXEC
        | MOUNT_ATTR_NOATIME
        | MOUNT_ATTR_STRICTATIME
        | MOUNT_ATTR_NODIRATIME
        | MOUNT_ATTR_NOSYMFOLLOW;
    if attrs & !supported != 0 {
        return_errno_with_message!(Errno::EINVAL, "unsupported mount attributes");
    }
    if attrs & MOUNT_ATTR_NOATIME != 0 && attrs & MOUNT_ATTR_STRICTATIME != 0 {
        return_errno_with_message!(Errno::EINVAL, "conflicting atime mount attributes");
    }

    let mut flags = PerMountFlags::default();
    if attrs & MOUNT_ATTR_RDONLY != 0 {
        flags |= PerMountFlags::RDONLY;
    }
    if attrs & MOUNT_ATTR_NOSUID != 0 {
        flags |= PerMountFlags::NOSUID;
    }
    if attrs & MOUNT_ATTR_NODEV != 0 {
        flags |= PerMountFlags::NODEV;
    }
    if attrs & MOUNT_ATTR_NOEXEC != 0 {
        flags |= PerMountFlags::NOEXEC;
    }
    if attrs & MOUNT_ATTR_NOATIME != 0 {
        flags |= PerMountFlags::NOATIME;
    }
    if attrs & MOUNT_ATTR_STRICTATIME != 0 {
        flags |= PerMountFlags::STRICTATIME;
    }
    if attrs & MOUNT_ATTR_NODIRATIME != 0 {
        flags |= PerMountFlags::NODIRATIME;
    }

    Ok(flags)
}

bitflags! {
    struct FsOpenFlags: u32 {
        const FSOPEN_CLOEXEC = 1;
    }

    struct FsMountFlags: u32 {
        const FSMOUNT_CLOEXEC = 1;
    }

    struct MoveMountFlags: u32 {
        const MOVE_MOUNT_F_EMPTY_PATH = 0x0000_0004;
    }
}

const FSCONFIG_SET_FLAG: u32 = 0;
const FSCONFIG_SET_STRING: u32 = 1;
const FSCONFIG_CMD_CREATE: u32 = 6;
const FSCONFIG_CMD_RECONFIGURE: u32 = 7;

const MOUNT_ATTR_RDONLY: u32 = 0x0000_0001;
const MOUNT_ATTR_NOSUID: u32 = 0x0000_0002;
const MOUNT_ATTR_NODEV: u32 = 0x0000_0004;
const MOUNT_ATTR_NOEXEC: u32 = 0x0000_0008;
const MOUNT_ATTR_NOATIME: u32 = 0x0000_0010;
const MOUNT_ATTR_STRICTATIME: u32 = 0x0000_0020;
const MOUNT_ATTR_NODIRATIME: u32 = 0x0000_0080;
const MOUNT_ATTR_NOSYMFOLLOW: u32 = 0x0020_0000;
