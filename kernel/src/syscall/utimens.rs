// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::{constants::MAX_FILENAME_LEN, SyscallReturn};
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        path::Dentry,
    },
    prelude::*,
    time::{clocks::RealTimeCoarseClock, timespec_t, timeval_t},
};

/// The 'sys_utimensat' system call sets the access and modification times of a file.
/// The times are defined by an array of two timespec structures, where times[0] represents the access time,
/// and times[1] represents the modification time.
/// The `flags` argument is a bit mask that can include the following values:
/// - `AT_SYMLINK_NOFOLLOW`: If set, the file is not dereferenced if it is a symbolic link.
pub fn sys_utimensat(
    dirfd: FileDesc,
    pathname_ptr: Vaddr,
    timespecs_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "utimensat: dirfd: {}, pathname_ptr: {:#x}, timespecs_ptr: {:#x}, flags: {:#x}",
        dirfd, pathname_ptr, timespecs_ptr, flags
    );
    let times = if timespecs_ptr != 0 {
        let (autime, mutime) = read_time_from_user::<timespec_t>(timespecs_ptr, ctx)?;
        if autime.is_utime_omit() && mutime.is_utime_omit() {
            return Ok(SyscallReturn::Return(0));
        }
        Some(TimeSpecPair {
            atime: autime,
            mtime: mutime,
        })
    } else {
        None
    };
    do_utimes(dirfd, pathname_ptr, times, flags, ctx)
}

/// The 'sys_futimesat' system call sets the access and modification times of a file.
/// Unlike 'sys_utimensat', it receives time values in the form of timeval structures,
/// and it does not support the 'flags' argument.
pub fn sys_futimesat(
    dirfd: FileDesc,
    pathname_ptr: Vaddr,
    timeval_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "futimesat: dirfd: {}, pathname_ptr: {:#x}, timeval_ptr: {:#x}",
        dirfd, pathname_ptr, timeval_ptr
    );
    do_futimesat(dirfd, pathname_ptr, timeval_ptr, ctx)
}

/// The 'sys_utimes' system call sets the access and modification times of a file.
/// It receives time values in the form of timeval structures like 'sys_futimesat',
/// but it uses the current working directory as the base directory.
pub fn sys_utimes(pathname_ptr: Vaddr, timeval_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!(
        "utimes: pathname_ptr: {:#x}, timeval_ptr: {:#x}",
        pathname_ptr, timeval_ptr
    );
    do_futimesat(AT_FDCWD, pathname_ptr, timeval_ptr, ctx)
}

/// The 'sys_utime' system call is similar to 'sys_utimes' but uses the older 'utimbuf' structure to specify times.
pub fn sys_utime(pathname_ptr: Vaddr, utimbuf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!(
        "utime: pathname_ptr: {:#x}, utimbuf_ptr: {:#x}",
        pathname_ptr, utimbuf_ptr
    );
    let times = if utimbuf_ptr != 0 {
        let utimbuf = ctx.user_space().read_val::<Utimbuf>(utimbuf_ptr)?;
        let atime = timespec_t {
            sec: utimbuf.actime,
            nsec: 0,
        };
        let mtime = timespec_t {
            sec: utimbuf.modtime,
            nsec: 0,
        };
        Some(TimeSpecPair { atime, mtime })
    } else {
        None
    };
    do_utimes(AT_FDCWD, pathname_ptr, times, 0, ctx)
}

// Structure to hold access and modification times
#[derive(Debug)]
struct TimeSpecPair {
    atime: timespec_t,
    mtime: timespec_t,
}

/// This struct is corresponding to the `utimbuf` struct in Linux.
#[repr(C)]
#[derive(Debug, Default, Copy, Clone, Pod)]
struct Utimbuf {
    actime: i64,
    modtime: i64,
}

fn vfs_utimes(dentry: &Dentry, times: Option<TimeSpecPair>) -> Result<SyscallReturn> {
    let (atime, mtime, ctime) = match times {
        Some(times) => {
            if !times.atime.is_valid() || !times.mtime.is_valid() {
                return_errno_with_message!(Errno::EINVAL, "invalid time")
            }
            let now = RealTimeCoarseClock::get().read_time();
            let atime = if times.atime.is_utime_omit() {
                dentry.atime()
            } else if times.atime.is_utime_now() {
                now
            } else {
                Duration::try_from(times.atime)?
            };
            let mtime = if times.mtime.is_utime_omit() {
                dentry.mtime()
            } else if times.mtime.is_utime_now() {
                now
            } else {
                Duration::try_from(times.mtime)?
            };
            (atime, mtime, now)
        }
        None => {
            let now = RealTimeCoarseClock::get().read_time();
            (now, now, now)
        }
    };

    // Update times
    dentry.set_atime(atime);
    dentry.set_mtime(mtime);
    dentry.set_ctime(ctime);

    Ok(SyscallReturn::Return(0))
}

// Common function to handle updating file times, supporting both fd and path based operations
fn do_utimes(
    dirfd: FileDesc,
    pathname_ptr: Vaddr,
    times: Option<TimeSpecPair>,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = UtimensFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;

    let pathname = if pathname_ptr == 0 {
        String::new()
    } else {
        let cstring = ctx
            .user_space()
            .read_cstring(pathname_ptr, MAX_FILENAME_LEN)?;
        cstring.to_string_lossy().into_owned()
    };
    let dentry = {
        // Determine the file system path and the corresponding entry
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        let fs = ctx.process.fs().read();
        if flags.contains(UtimensFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };

    vfs_utimes(&dentry, times)
}

// Sets the access and modification times for a file,
// specified by a pathname relative to the directory file descriptor `dirfd`.
fn do_futimesat(
    dirfd: FileDesc,
    pathname_ptr: Vaddr,
    timeval_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let times = if timeval_ptr != 0 {
        let (autime, mutime) = read_time_from_user::<timeval_t>(timeval_ptr, ctx)?;
        if autime.usec >= 1000000
            || autime.usec < 0
            || autime.sec < 0
            || mutime.usec >= 1000000
            || mutime.usec < 0
            || mutime.sec < 0
        {
            return_errno_with_message!(Errno::EINVAL, "Invalid time");
        }
        let (autime, mutime) = (timespec_t::from(autime), timespec_t::from(mutime));
        Some(TimeSpecPair {
            atime: autime,
            mtime: mutime,
        })
    } else {
        None
    };
    do_utimes(dirfd, pathname_ptr, times, 0, ctx)
}

fn read_time_from_user<T: Pod>(time_ptr: Vaddr, ctx: &Context) -> Result<(T, T)> {
    let mut time_addr = time_ptr;
    let user_space = ctx.user_space();
    let autime = user_space.read_val::<T>(time_addr)?;
    time_addr += core::mem::size_of::<T>();
    let mutime = user_space.read_val::<T>(time_addr)?;
    Ok((autime, mutime))
}

trait UtimeExt {
    fn is_utime_now(&self) -> bool;
    fn is_utime_omit(&self) -> bool;
    fn is_valid(&self) -> bool;
}

impl UtimeExt for timespec_t {
    fn is_utime_now(&self) -> bool {
        self.nsec == UTIME_NOW
    }

    fn is_utime_omit(&self) -> bool {
        self.nsec == UTIME_OMIT
    }

    fn is_valid(&self) -> bool {
        self.nsec == UTIME_OMIT
            || self.nsec == UTIME_NOW
            || (self.nsec >= 0 && self.nsec <= 999_999_999)
    }
}

const UTIME_NOW: i64 = (1i64 << 30) - 1i64;
const UTIME_OMIT: i64 = (1i64 << 30) - 2i64;

bitflags::bitflags! {
    struct UtimensFlags: u32 {
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
