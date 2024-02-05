// SPDX-License-Identifier: MPL-2.0

use crate::fs::{file_table::FileDescripter, fs_resolver::FsPath};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::time::timespec_t;
use crate::util::{read_cstring_from_user, read_val_from_user};
use core::time::Duration;

use super::SyscallReturn;
use super::SYS_UTIMENSAT;

pub fn sys_utimensat(
    dirfd: FileDescripter,
    pathname_ptr: Vaddr,
    timespecs_ptr: Vaddr,
    flags: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_UTIMENSAT);
    let pathname = read_cstring_from_user(pathname_ptr, MAX_FILENAME_LEN)?;
    let (atime, mtime) = {
        let (autime, mutime) = if timespecs_ptr == 0 {
            (timespec_t::utime_now(), timespec_t::utime_now())
        } else {
            let mut timespecs_addr = timespecs_ptr;
            let autime = read_val_from_user::<timespec_t>(timespecs_addr)?;
            timespecs_addr += core::mem::size_of::<timespec_t>();
            let mutime = read_val_from_user::<timespec_t>(timespecs_addr)?;
            (autime, mutime)
        };

        // TODO: Get current time
        let current_time: timespec_t = Default::default();

        let atime = if autime.is_utime_omit() {
            None
        } else if autime.is_utime_now() {
            Some(current_time)
        } else {
            Some(autime)
        };
        let mtime = if mutime.is_utime_omit() {
            None
        } else if mutime.is_utime_now() {
            Some(current_time)
        } else {
            Some(mutime)
        };
        (atime, mtime)
    };
    let flags = UtimensFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, pathname = {:?}, atime = {:?}, mtime = {:?}, flags = {:?}",
        dirfd, pathname, atime, mtime, flags
    );

    if atime.is_none() && mtime.is_none() {
        return Ok(SyscallReturn::Return(0));
    }
    let current = current!();
    let dentry = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "pathname is empty");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        let fs = current.fs().read();
        if flags.contains(UtimensFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };
    if let Some(time) = atime {
        dentry.set_atime(Duration::from(time));
    }
    if let Some(time) = mtime {
        dentry.set_mtime(Duration::from(time));
    }
    Ok(SyscallReturn::Return(0))
}

trait UtimeExt {
    fn utime_now() -> Self;
    fn utime_omit() -> Self;
    fn is_utime_now(&self) -> bool;
    fn is_utime_omit(&self) -> bool;
}

impl UtimeExt for timespec_t {
    fn utime_now() -> Self {
        Self {
            sec: 0,
            nsec: UTIME_NOW,
        }
    }

    fn utime_omit() -> Self {
        Self {
            sec: 0,
            nsec: UTIME_OMIT,
        }
    }

    fn is_utime_now(&self) -> bool {
        self.nsec == UTIME_NOW
    }

    fn is_utime_omit(&self) -> bool {
        self.nsec == UTIME_OMIT
    }
}

const UTIME_NOW: i64 = (1i64 << 30) - 1i64;
const UTIME_OMIT: i64 = (1i64 << 30) - 2i64;

bitflags::bitflags! {
    struct UtimensFlags: u32 {
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
