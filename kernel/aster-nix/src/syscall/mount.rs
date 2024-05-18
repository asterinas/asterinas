// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        exfat::{ExfatFS, ExfatMountOptions},
        ext2::Ext2,
        fs_resolver::{FsPath, AT_FDCWD},
        path::Dentry,
        utils::{FileSystem, InodeType},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

/// The data argument is interpreted by the different filesystems.
/// Typically it is a string of comma-separated options understood by
/// this filesystem. The current implementation only considers the case
/// where it is NULL. Because it should be interpreted by the specific filesystems.
pub fn sys_mount(
    devname_addr: Vaddr,
    dirname_addr: Vaddr,
    fstype_addr: Vaddr,
    flags: u64,
    data: Vaddr,
) -> Result<SyscallReturn> {
    let devname = read_cstring_from_user(devname_addr, MAX_FILENAME_LEN)?;
    let dirname = read_cstring_from_user(dirname_addr, MAX_FILENAME_LEN)?;
    let mount_flags = MountFlags::from_bits_truncate(flags as u32);
    debug!(
        "devname = {:?}, dirname = {:?}, fstype = 0x{:x}, flags = {:?}, data = 0x{:x}",
        devname, dirname, fstype_addr, mount_flags, data,
    );

    let current = current!();
    let target_dentry = {
        let dirname = dirname.to_string_lossy();
        if dirname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "dirname is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, dirname.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };

    if mount_flags.contains(MountFlags::MS_REMOUNT) && mount_flags.contains(MountFlags::MS_BIND) {
        do_reconfigure_mnt()?;
    } else if mount_flags.contains(MountFlags::MS_REMOUNT) {
        do_remount()?;
    } else if mount_flags.contains(MountFlags::MS_BIND) {
        do_loopback(
            devname.clone(),
            target_dentry.clone(),
            mount_flags.contains(MountFlags::MS_REC),
        )?;
    } else if mount_flags.contains(MountFlags::MS_SHARED)
        | mount_flags.contains(MountFlags::MS_PRIVATE)
        | mount_flags.contains(MountFlags::MS_SLAVE)
        | mount_flags.contains(MountFlags::MS_UNBINDABLE)
    {
        do_change_type()?;
    } else if mount_flags.contains(MountFlags::MS_MOVE) {
        do_move_mount_old(devname, target_dentry)?;
    } else {
        do_new_mount(devname, fstype_addr, target_dentry)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn do_reconfigure_mnt() -> Result<()> {
    todo!()
}

fn do_remount() -> Result<()> {
    todo!()
}

/// Bind a mount to a new location.
fn do_loopback(old_name: CString, new_dentry: Arc<Dentry>, recursive: bool) -> Result<()> {
    let current = current!();
    let old_dentry = {
        let old_name = old_name.to_string_lossy();
        if old_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "old_name is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, old_name.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };

    if old_dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "old_name must be directory");
    };

    let new_mount = old_dentry.do_loopback(recursive);
    new_mount.graft_mount_node_tree(new_dentry.clone())?;
    Ok(())
}

fn do_change_type() -> Result<()> {
    todo!()
}

/// Move a mount from old location to new location.
fn do_move_mount_old(old_name: CString, new_dentry: Arc<Dentry>) -> Result<()> {
    let current = current!();
    let old_dentry = {
        let old_name = old_name.to_string_lossy();
        if old_name.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "old_name is empty");
        }
        let fs_path = FsPath::new(AT_FDCWD, old_name.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };

    if !old_dentry.is_root_of_mount() {
        return_errno_with_message!(Errno::EINVAL, "old_name can not be moved");
    };
    if old_dentry.mount_node().parent().is_none() {
        return_errno_with_message!(Errno::EINVAL, "old_name can not be moved");
    }

    old_dentry
        .mount_node()
        .graft_mount_node_tree(new_dentry.clone())?;

    Ok(())
}

/// Mount a new filesystem.
fn do_new_mount(devname: CString, fs_type: Vaddr, target_dentry: Arc<Dentry>) -> Result<()> {
    if target_dentry.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "mountpoint must be directory");
    };

    let fs_type = read_cstring_from_user(fs_type, MAX_FILENAME_LEN)?;
    if fs_type.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "fs_type is empty");
    }
    let fs = get_fs(fs_type, devname)?;
    target_dentry.mount(fs)?;
    Ok(())
}

/// Get the filesystem by fs_type and devname.
fn get_fs(fs_type: CString, devname: CString) -> Result<Arc<dyn FileSystem>> {
    let devname = devname.to_str().unwrap();
    let device = match aster_block::get_device(devname) {
        Some(device) => device,
        None => return_errno_with_message!(Errno::ENOENT, "Device does not exist"),
    };
    let fs_type = fs_type.to_str().unwrap();
    match fs_type {
        "ext2" => {
            let ext2_fs = Ext2::open(device)?;
            Ok(ext2_fs)
        }
        "exfat" => {
            let exfat_fs = ExfatFS::open(device, ExfatMountOptions::default())?;
            Ok(exfat_fs)
        }
        _ => return_errno_with_message!(Errno::EINVAL, "Invalid fs type"),
    }
}

bitflags! {
    struct MountFlags: u32 {
        const MS_RDONLY = 1;              /* Mount read-only */
        const MS_NOSUID = 1<<1;           /* Ignore suid and sgid bits */
        const MS_NODEV = 1<<2;            /* Disallow access to device special files */
        const MS_NOEXEC = 1<<3;           /* Disallow program execution */
        const MS_SYNCHRONOUS = 1<<4;      /* Writes are synced at once */
        const MS_REMOUNT = 1<<5;          /* Alter flags of a mounted FS */
        const MS_MANDLOCK = 1<<6;         /* Allow mandatory locks on an FS */
        const MS_DIRSYNC = 1<<7;          /* Directory modifications are synchronous */
        const MS_NOSYMFOLLOW = 1<<8;      /* Do not follow symlinks */
        const MS_NOATIME = 1<<10;         /* Do not update access times. */
        const MS_NODIRATIME = 1<<11;      /* Do not update directory access times. */
        const MS_BIND = 1<<12;            /* Bind directory at different place. */
        const MS_MOVE = 1<<13;            /* Move mount from old to new. */
        const MS_REC = 1<<14;             /* Create recursive mount. */
        const MS_SILENT = 1<<15;          /* Suppress certain messages in kernel log. */
        const MS_POSIXACL = 1<<16;        /* VFS does not apply the umask. */
        const MS_UNBINDABLE = 1<<17;      /* Change to unbindable. */
        const MS_PRIVATE = 1<<18; 	      /* Change to private. */
        const MS_SLAVE = 1<<19;           /* Change to slave. */
        const MS_SHARED = 1<<20;          /* Change to shared. */
        const MS_RELATIME = 1<<21; 	      /* Update atime relative to mtime/ctime. */
        const MS_KERNMOUNT = 1<<22;       /* This is a kern_mount call. */
    }
}
