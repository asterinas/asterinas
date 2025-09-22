// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        fs_resolver::{FsPath, AT_FDCWD},
        path::{AtimePolicy, Mount, MountOptions, Path},
        registry::FsProperties,
        utils::{FileSystem, FsMountOptions, InodeType},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

/// The `data` argument is interpreted by the different filesystems.
/// Typically it is a string of comma-separated options understood by
/// this filesystem. The current implementation only considers the case
/// where it is `NULL`. Because it should be interpreted by the specific filesystems.
pub fn sys_mount(
    devname_addr: Vaddr,
    dirname_addr: Vaddr,
    fstype_addr: Vaddr,
    flags: u64,
    data: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let devname = user_space.read_cstring(devname_addr, MAX_FILENAME_LEN)?;
    let dirname = user_space.read_cstring(dirname_addr, MAX_FILENAME_LEN)?;
    let mount_flags = MountFlags::from_bits_truncate(flags as u32);
    debug!(
        "devname = {:?}, dirname = {:?}, fstype = 0x{:x}, flags = {:?}, data = 0x{:x}",
        devname, dirname, fstype_addr, mount_flags, data,
    );

    let dst_path = {
        let dirname = dirname.to_string_lossy();
        if dirname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "dirname is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, dirname.as_ref())?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    if mount_flags.contains(MountFlags::MS_REMOUNT) && mount_flags.contains(MountFlags::MS_BIND) {
        do_remount_mnt(dst_path.mount_node(), mount_flags, ctx)?
    } else if mount_flags.contains(MountFlags::MS_REMOUNT) {
        do_remount_mnt(dst_path.mount_node(), mount_flags, ctx)?;
        do_remount_fs(&dst_path.fs(), mount_flags, data, ctx)?;
    } else if mount_flags.contains(MountFlags::MS_BIND) {
        do_bind_mount(
            devname,
            dst_path,
            mount_flags.contains(MountFlags::MS_REC),
            ctx,
        )?;
    } else if mount_flags.contains(MountFlags::MS_SHARED)
        | mount_flags.contains(MountFlags::MS_PRIVATE)
        | mount_flags.contains(MountFlags::MS_SLAVE)
        | mount_flags.contains(MountFlags::MS_UNBINDABLE)
    {
        do_change_type()?;
    } else if mount_flags.contains(MountFlags::MS_MOVE) {
        do_move_mount_old(devname, dst_path, ctx)?;
    } else {
        do_new_mount(devname, mount_flags, fstype_addr, dst_path, data, ctx)?;
    }

    Ok(SyscallReturn::Return(0))
}

/// Remount the mount with new flags.
fn do_remount_mnt(mount: &Mount, flags: MountFlags, _ctx: &Context) -> Result<()> {
    let mut new_options: MountOptions = flags.into();
    let specified_atime_policy = flags.contains(MountFlags::MS_STRICTATIME)
        || flags.contains(MountFlags::MS_NOATIME)
        || flags.contains(MountFlags::MS_RELATIME);

    let mut old_options = mount.options();
    if !specified_atime_policy {
        // Preserve the existing atime policy if none is specified in the flags
        new_options.set_atime_policy(old_options.atime_policy());
    }
    *old_options = new_options;

    Ok(())
}

/// Remount the filesystem with new flags and data.
fn do_remount_fs(
    fs: &Arc<dyn FileSystem>,
    flags: MountFlags,
    data: Vaddr,
    ctx: &Context,
) -> Result<()> {
    let fs_mount_options: FsMountOptions = flags.into();
    fs.set_mount_options(fs_mount_options, data, ctx)?;

    Ok(())
}

/// Bind a mount to a dst location.
///
/// If recursive is true, then bind the mount recursively.
/// Such as use user command `mount --rbind src dst`.
fn do_bind_mount(src_name: CString, dst_path: Path, recursive: bool, ctx: &Context) -> Result<()> {
    let src_path = {
        let src_name = src_name.to_string_lossy();
        if src_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "src_name is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, src_name.as_ref())?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    if src_path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "src_name must be directory");
    };

    src_path.bind_mount_to(&dst_path, recursive)?;
    Ok(())
}

fn do_change_type() -> Result<()> {
    return_errno_with_message!(Errno::EINVAL, "do_change_type is not supported");
}

/// Move a mount from src location to dst location.
fn do_move_mount_old(src_name: CString, dst_path: Path, ctx: &Context) -> Result<()> {
    let src_path = {
        let src_name = src_name.to_string_lossy();
        if src_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "src_name is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, src_name.as_ref())?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    src_path.move_mount_to(&dst_path)?;

    Ok(())
}

/// Mount a new filesystem.
fn do_new_mount(
    devname: CString,
    flags: MountFlags,
    fs_type: Vaddr,
    target_path: Path,
    data: Vaddr,
    ctx: &Context,
) -> Result<()> {
    if target_path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "mountpoint must be directory");
    };

    let fs_type = ctx.user_space().read_cstring(fs_type, MAX_FILENAME_LEN)?;
    if fs_type.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "fs_type is empty");
    }
    let fs = get_fs(fs_type, devname, data, ctx)?;
    fs.set_mount_options(flags.into(), data, ctx)?;

    let new_mount = target_path.mount(fs)?;
    let new_mount_options: MountOptions = flags.into();
    *new_mount.options() = new_mount_options;

    Ok(())
}

/// Get the filesystem by fs_type and devname.
fn get_fs(
    fs_type: CString,
    devname: CString,
    data: Vaddr,
    ctx: &Context,
) -> Result<Arc<dyn FileSystem>> {
    let user_space = ctx.user_space();
    let data = if data == 0 {
        None
    } else {
        Some(user_space.read_cstring(data, MAX_FILENAME_LEN)?)
    };

    let fs_type = fs_type
        .to_str()
        .map_err(|_| Error::with_message(Errno::ENODEV, "Invalid file system type"))?;
    let fs_type = crate::fs::registry::look_up(fs_type)
        .ok_or(Error::with_message(Errno::EINVAL, "Invalid fs type"))?;

    let disk = if fs_type.properties().contains(FsProperties::NEED_DISK) {
        Some(
            aster_block::get_device(devname.to_str().unwrap())
                .ok_or(Error::with_message(Errno::ENOENT, "device does not exist"))?,
        )
    } else {
        None
    };

    fs_type.create(data, disk)
}

bitflags! {
    struct MountFlags: u32 {
        const MS_RDONLY        =   1 << 0;       // Mount read-only.
        const MS_NOSUID        =   1 << 1;       // Ignore suid and sgid bits.
        const MS_NODEV         =   1 << 2;       // Disallow access to device special files.
        const MS_NOEXEC        =   1 << 3;       // Disallow program execution.
        const MS_SYNCHRONOUS   =   1 << 4;       // Writes are synced at once.
        const MS_REMOUNT       =   1 << 5;       // Alter flags of a mounted FS.
        const MS_MANDLOCK      =   1 << 6;       // Allow mandatory locks on an FS.
        const MS_DIRSYNC       =   1 << 7;       // Directory modifications are synchronous.
        const MS_NOSYMFOLLOW   =   1 << 8;       // Do not follow symlinks.
        const MS_NOATIME       =   1 << 10;      // Do not update access times.
        const MS_NODIRATIME    =   1 << 11;      // Do not update directory access times.
        const MS_BIND          =   1 << 12;      // Bind directory at different place.
        const MS_MOVE          =   1 << 13;      // Move mount from old to new.
        const MS_REC           =   1 << 14;      // Create recursive mount.
        const MS_SILENT        =   1 << 15;      // Suppress certain messages in kernel log.
        const MS_POSIXACL      =   1 << 16;      // VFS does not apply the umask.
        const MS_UNBINDABLE    =   1 << 17;      // Change to unbindable.
        const MS_PRIVATE       =   1 << 18; 	 // Change to private.
        const MS_SLAVE         =   1 << 19;      // Change to slave.
        const MS_SHARED        =   1 << 20;      // Change to shared.
        const MS_RELATIME      =   1 << 21; 	 // Update atime relative to mtime/ctime.
        const MS_KERNMOUNT     =   1 << 22;      // This is a kern_mount call.
        const MS_STRICTATIME   =   1 << 24; 	 // Always perform atime updates.
        const MS_LAZYTIME      =   1 << 25; 	 // Update the on-disk [acm]times lazily.
    }
}

impl From<MountFlags> for MountOptions {
    fn from(flags: MountFlags) -> Self {
        let mut options = MountOptions::default();
        if flags.contains(MountFlags::MS_NOSUID) {
            options.set_nosuid();
        }
        if flags.contains(MountFlags::MS_NODEV) {
            options.set_nodev();
        }
        if flags.contains(MountFlags::MS_NOEXEC) {
            options.set_noexec();
        }
        if flags.contains(MountFlags::MS_RDONLY) {
            options.set_rdonly();
        }
        if flags.contains(MountFlags::MS_NODIRATIME) {
            options.set_nodiratime();
        }

        if flags.contains(MountFlags::MS_STRICTATIME) {
            options.set_atime_policy(AtimePolicy::Strictatime);
        } else if flags.contains(MountFlags::MS_NOATIME) {
            options.set_atime_policy(AtimePolicy::Noatime);
        }

        options
    }
}

impl From<MountFlags> for FsMountOptions {
    fn from(flags: MountFlags) -> Self {
        let mut options = FsMountOptions::empty();
        if flags.contains(MountFlags::MS_RDONLY) {
            options |= FsMountOptions::READONLY;
        }
        if flags.contains(MountFlags::MS_SYNCHRONOUS) {
            options |= FsMountOptions::SYNCHRONOUS;
        }
        if flags.contains(MountFlags::MS_MANDLOCK) {
            options |= FsMountOptions::MANDLOCK;
        }
        if flags.contains(MountFlags::MS_DIRSYNC) {
            options |= FsMountOptions::DIRSYNC;
        }
        if flags.contains(MountFlags::MS_SILENT) {
            options |= FsMountOptions::SILENT;
        }
        if flags.contains(MountFlags::MS_LAZYTIME) {
            options |= FsMountOptions::LAZYTIME;
        }

        options
    }
}
