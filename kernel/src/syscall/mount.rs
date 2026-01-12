// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use super::SyscallReturn;
use crate::{
    fs::{
        path::{AT_FDCWD, FsPath, MountPropType, Path, PerMountFlags},
        registry::FsProperties,
        utils::{FileSystem, FsFlags, InodeType},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_mount(
    src_name_addr: Vaddr,
    dst_name_addr: Vaddr,
    fs_type_addr: Vaddr,
    flags: u64,
    // The `data` argument is interpreted by the different filesystems.
    // Typically it is a string of comma-separated options understood by
    // this filesystem. The current implementation only considers the case
    // where it is `NULL`. Because it should be interpreted by the specific filesystems.
    data_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let dst_name = ctx
        .user_space()
        .read_cstring(dst_name_addr, MAX_FILENAME_LEN)?;
    let mount_flags = MountFlags::from_bits_truncate(flags as u32);
    debug!(
        "src_name_addr = 0x{:x}, dst_name = {:?}, fstype = 0x{:x}, flags = {:?}, data_addr = 0x{:x}",
        src_name_addr, dst_name, fs_type_addr, mount_flags, data_addr,
    );

    let dst_path = {
        let dst_name = dst_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(AT_FDCWD, &dst_name)?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    if mount_flags.contains(MountFlags::MS_REMOUNT) && mount_flags.contains(MountFlags::MS_BIND) {
        // If `MS_BIND` is specified, only the mount flags are changed.
        do_remount_mnt(&dst_path, mount_flags, ctx)?;
    } else if mount_flags.contains(MountFlags::MS_REMOUNT) {
        do_remount_mnt_and_fs(&dst_path, mount_flags, data_addr, ctx)?;
    } else if mount_flags.contains(MountFlags::MS_BIND) {
        do_bind_mount(
            src_name_addr,
            dst_path,
            mount_flags.contains(MountFlags::MS_REC),
            ctx,
        )?;
    } else if mount_flags.intersects(MS_PROPAGATION) {
        do_change_type(dst_path, mount_flags, ctx)?;
    } else if mount_flags.contains(MountFlags::MS_MOVE) {
        do_move_mount_old(src_name_addr, dst_path, ctx)?;
    } else {
        do_new_mount(
            src_name_addr,
            mount_flags,
            fs_type_addr,
            dst_path,
            data_addr,
            ctx,
        )?;
    }

    Ok(SyscallReturn::Return(0))
}

/// Remounts the mount with new flags.
fn do_remount_mnt(path: &Path, flags: MountFlags, ctx: &Context) -> Result<()> {
    let per_mount_flags = PerMountFlags::from(flags);

    path.remount(per_mount_flags, None, None, ctx)
}

/// Remounts the filesystem with new flags and data.
fn do_remount_mnt_and_fs(
    path: &Path,
    flags: MountFlags,
    data_addr: Vaddr,
    ctx: &Context,
) -> Result<()> {
    let per_mount_flags = PerMountFlags::from(flags);
    let fs_flags = FsFlags::from(flags);
    let data = if data_addr == 0 {
        None
    } else {
        Some(ctx.user_space().read_cstring(data_addr, MAX_FILENAME_LEN)?)
    };

    path.remount(per_mount_flags, Some(fs_flags), data, ctx)
}

/// Binds a mount to a dst location.
///
/// If recursive is true, then bind the mount recursively.
/// Such as use user command `mount --rbind src dst`.
fn do_bind_mount(
    src_name_addr: Vaddr,
    dst_path: Path,
    recursive: bool,
    ctx: &Context,
) -> Result<()> {
    let src_path = {
        let src_name = ctx
            .user_space()
            .read_cstring(src_name_addr, MAX_FILENAME_LEN)?;
        let src_name = src_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(AT_FDCWD, &src_name)?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    src_path.bind_mount_to(&dst_path, recursive, ctx)?;
    Ok(())
}

// All valid propagation flags.
const MS_PROPAGATION: MountFlags = MountFlags::MS_SHARED
    .union(MountFlags::MS_PRIVATE)
    .union(MountFlags::MS_SLAVE)
    .union(MountFlags::MS_UNBINDABLE);

fn do_change_type(target_path: Path, flags: MountFlags, ctx: &Context) -> Result<()> {
    // All flags that are allowed during a propagation change.
    const ALLOWED_FLAGS: MountFlags = MS_PROPAGATION
        .union(MountFlags::MS_REC)
        .union(MountFlags::MS_SILENT);

    if !(flags & !ALLOWED_FLAGS).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the mount propagation flags are unsupported");
    }

    let propagation_flags = flags & MS_PROPAGATION;
    if propagation_flags.bits().count_ones() > 1 {
        return_errno_with_message!(
            Errno::EINVAL,
            "mount flags includes more than one of MS_SHARED, MS_PRIVATE, MS_SLAVE, or MS_UNBINDABLE"
        );
    }

    if flags.contains(MountFlags::MS_PRIVATE) {
        let recursive = flags.contains(MountFlags::MS_REC);
        target_path.set_mount_propagation(MountPropType::Private, recursive, ctx)?;
        Ok(())
    } else {
        return_errno_with_message!(Errno::EINVAL, "the mount propagation type is unsupported");
    }
}

/// Moves a mount from src location to dst location.
fn do_move_mount_old(src_name_addr: Vaddr, dst_path: Path, ctx: &Context) -> Result<()> {
    let src_path = {
        let src_name = ctx
            .user_space()
            .read_cstring(src_name_addr, MAX_FILENAME_LEN)?;
        let src_name = src_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(AT_FDCWD, &src_name)?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?
    };

    src_path.move_mount_to(&dst_path, ctx)?;

    Ok(())
}

/// Mounts a new filesystem.
fn do_new_mount(
    src_name_addr: Vaddr,
    flags: MountFlags,
    fs_type_addr: Vaddr,
    target_path: Path,
    data_addr: Vaddr,
    ctx: &Context,
) -> Result<()> {
    if target_path.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "mountpoint must be directory");
    };

    let fs_type = ctx
        .user_space()
        .read_cstring(fs_type_addr, MAX_FILENAME_LEN)?;
    if fs_type.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "fs_type is empty");
    }
    let fs = get_fs(src_name_addr, flags, fs_type, data_addr, ctx)?;
    target_path.mount(fs, flags.into(), ctx)?;
    Ok(())
}

/// Gets the filesystem by fs_type and devname.
fn get_fs(
    src_name_addr: Vaddr,
    flags: MountFlags,
    fs_type: CString,
    data_addr: Vaddr,
    ctx: &Context,
) -> Result<Arc<dyn FileSystem>> {
    let user_space = ctx.user_space();
    let data = if data_addr == 0 {
        None
    } else {
        Some(user_space.read_cstring(data_addr, MAX_FILENAME_LEN)?)
    };

    let fs_type = fs_type
        .to_str()
        .map_err(|_| Error::with_message(Errno::ENODEV, "invalid file system type"))?;
    let fs_type = crate::fs::registry::look_up(fs_type).ok_or(Error::with_message(
        Errno::ENODEV,
        "the filesystem is not configured in the kernel",
    ))?;

    let disk = if fs_type.properties().contains(FsProperties::NEED_DISK) {
        let devname = user_space.read_cstring(src_name_addr, MAX_FILENAME_LEN)?;
        let path = devname.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(AT_FDCWD, path.as_ref())?;
        let path = ctx
            .thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?;
        if !path.type_().is_device() {
            return_errno_with_message!(Errno::ENODEV, "the path is not a device file");
        }

        let id = DeviceId::from_encoded_u64(path.metadata().rdev);
        let device = id.and_then(aster_block::lookup);
        if device.is_none() {
            return_errno_with_message!(Errno::ENODEV, "the device is not found");
        }

        device
    } else {
        None
    };

    fs_type.create(flags.into(), data, disk)
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

impl From<MountFlags> for PerMountFlags {
    fn from(flags: MountFlags) -> Self {
        Self::from_bits_truncate(flags.bits())
    }
}

impl From<MountFlags> for FsFlags {
    fn from(flags: MountFlags) -> Self {
        Self::from_bits_truncate(flags.bits())
    }
}
