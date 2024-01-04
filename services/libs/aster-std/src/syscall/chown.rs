use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::PATH_MAX,
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{Gid, Uid};
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::{SYS_FCHOWN, SYS_FCHOWNAT};

pub fn sys_fchown(fd: FileDescripter, uid: u32, gid: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHOWN);
    debug!("fd = {}, uid = {}, gid = {}", fd, uid, gid);

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    file.set_owner(Uid::new(uid), Gid::new(gid))?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chown(path_ptr: Vaddr, uid: u32, gid: u32) -> Result<SyscallReturn> {
    self::sys_fchownat(AT_FDCWD, path_ptr, uid, gid, 0)
}

pub fn sys_lchown(path_ptr: Vaddr, uid: u32, gid: u32) -> Result<SyscallReturn> {
    self::sys_fchownat(
        AT_FDCWD,
        path_ptr,
        uid,
        gid,
        ChownFlags::AT_SYMLINK_NOFOLLOW.bits(),
    )
}

pub fn sys_fchownat(
    dirfd: FileDescripter,
    path_ptr: Vaddr,
    uid: u32,
    gid: u32,
    flags: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHOWNAT);
    let path = read_cstring_from_user(path_ptr, PATH_MAX)?;
    let flags = ChownFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, path = {:?}, uid = {}, gid = {}, flags = {:?}",
        dirfd, path, uid, gid, flags
    );

    let current = current!();
    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs = current.fs().read();
        if flags.contains(ChownFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };
    dentry.set_inode_owner(Uid::new(uid), Gid::new(gid));
    Ok(SyscallReturn::Return(0))
}

bitflags! {
    struct ChownFlags: u32 {
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
