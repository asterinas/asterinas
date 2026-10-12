// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::mm::VmIo;

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs,
    fs::{
        file::{
            StatusFlags,
            file_table::{RawFileDesc, get_file_fast},
        },
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath, Path},
    },
    prelude::*,
    time::{clocks::RealTimeCoarseClock, timespec_t, timeval_t},
};

/// The `sys_utimensat` system call sets the access and modification times of a file.
///
/// The times are defined by an array of two `timespec` structures, where `times[0]` represents the
/// access time and `times[1]` represents the modification time.
///
/// The `flags` argument is a bit mask that can include the following values:
/// - `AT_SYMLINK_NOFOLLOW`: If set, the file is not dereferenced if it is a symbolic link.
/// - `AT_EMPTY_PATH`: If set, an empty pathname means operating on `dirfd` directly.
pub(super) fn sys_utimensat(
    dirfd: RawFileDesc,
    pathname_ptr: Vaddr,
    timespecs_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "utimensat: dirfd = {}, pathname_ptr = {:#x}, timespecs_ptr = {:#x}, flags = {:#x}",
        dirfd, pathname_ptr, timespecs_ptr, flags
    );
    let times = if timespecs_ptr != 0 {
        let (autime, mutime) = read_time_from_user::<timespec_t>(timespecs_ptr, ctx)?;
        let atime = TimeSpec::try_from(autime)?;
        let mtime = TimeSpec::try_from(mutime)?;
        if matches!(atime, TimeSpec::Omit) && matches!(mtime, TimeSpec::Omit) {
            return Ok(SyscallReturn::Return(0));
        }
        Some(TimeSpecPair { atime, mtime })
    } else {
        None
    };
    do_utimes(dirfd, pathname_ptr, times, flags, ctx)
}

/// The `sys_futimesat` system call sets the access and modification times of a file.
///
/// Unlike `sys_utimensat`, it receives time values in the form of `timeval` structures and it does
/// not support the `flags` argument.
pub(super) fn sys_futimesat(
    dirfd: RawFileDesc,
    pathname_ptr: Vaddr,
    timeval_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "futimesat: dirfd = {}, pathname_ptr = {:#x}, timeval_ptr = {:#x}",
        dirfd, pathname_ptr, timeval_ptr
    );
    do_futimesat(dirfd, pathname_ptr, timeval_ptr, ctx)
}

/// The `sys_utimes` system call sets the access and modification times of a file.
///
/// Unlike `sys_utimensat`, it receives time values in the form of `timeval` structures and it uses
/// the current working directory as the base directory.
pub(super) fn sys_utimes(
    pathname_ptr: Vaddr,
    timeval_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "utimes: pathname_ptr = {:#x}, timeval_ptr = {:#x}",
        pathname_ptr, timeval_ptr
    );
    do_futimesat(AT_FDCWD, pathname_ptr, timeval_ptr, ctx)
}

/// Sets the access and modification times for a file, specified by a pathname relative to the
/// directory file descriptor `dirfd`.
fn do_futimesat(
    dirfd: RawFileDesc,
    pathname_ptr: Vaddr,
    timeval_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let times = if timeval_ptr != 0 {
        let (autime, mutime) = read_time_from_user::<timeval_t>(timeval_ptr, ctx)?;
        Some(TimeSpecPair {
            atime: TimeSpec::try_from(autime)?,
            mtime: TimeSpec::try_from(mutime)?,
        })
    } else {
        None
    };
    do_utimes(dirfd, pathname_ptr, times, 0, ctx)
}

fn read_time_from_user<T: Pod>(time_ptr: Vaddr, ctx: &Context) -> Result<(T, T)> {
    let user_space = ctx.user_space();

    let mut time_addr = time_ptr;
    let autime = user_space.read_val::<T>(time_addr)?;
    time_addr += size_of::<T>();
    let mutime = user_space.read_val::<T>(time_addr)?;

    Ok((autime, mutime))
}

/// The `sys_utime` system call sets the access and modification times of a file.
///
/// The `sys_utime` system call is similar to the `sys_utimes` system call, but it uses the older
/// `utimbuf` structure to specify times.
pub(super) fn sys_utime(
    pathname_ptr: Vaddr,
    utimbuf_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "utime: pathname_ptr = {:#x}, utimbuf_ptr = {:#x}",
        pathname_ptr, utimbuf_ptr
    );
    let times = if utimbuf_ptr != 0 {
        let utimbuf = ctx.user_space().read_val::<Utimbuf>(utimbuf_ptr)?;
        let autime = timespec_t {
            sec: utimbuf.actime,
            nsec: 0,
        };
        let mutime = timespec_t {
            sec: utimbuf.modtime,
            nsec: 0,
        };
        Some(TimeSpecPair {
            atime: TimeSpec::try_from(autime)?,
            mtime: TimeSpec::try_from(mutime)?,
        })
    } else {
        None
    };
    do_utimes(AT_FDCWD, pathname_ptr, times, 0, ctx)
}

/// This struct is corresponding to the `utimbuf` struct in Linux.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct Utimbuf {
    actime: i64,
    modtime: i64,
}

// ============================== Common logic ==============================

/// A structure to hold access and modification times.
#[derive(Debug)]
struct TimeSpecPair {
    atime: TimeSpec,
    mtime: TimeSpec,
}

/// An enum to hold access or modification time.
#[derive(Debug)]
enum TimeSpec {
    Now,
    Omit,
    Spec(Duration),
}

impl TimeSpec {
    /// Converts to a duration, using `now` as [`Self::Now`].
    fn with_now(self, now: Duration) -> Option<Duration> {
        match self {
            Self::Now => Some(now),
            Self::Omit => None,
            Self::Spec(duration) => Some(duration),
        }
    }
}

impl TryFrom<timeval_t> for TimeSpec {
    type Error = Error;

    fn try_from(value: timeval_t) -> Result<Self> {
        Ok(Self::Spec(Duration::try_from(value)?))
    }
}

impl TryFrom<timespec_t> for TimeSpec {
    type Error = Error;

    fn try_from(value: timespec_t) -> Result<Self> {
        if value.nsec == UTIME_NOW {
            Ok(Self::Now)
        } else if value.nsec == UTIME_OMIT {
            Ok(Self::Omit)
        } else {
            Ok(Self::Spec(Duration::try_from(value)?))
        }
    }
}

const UTIME_NOW: i64 = (1i64 << 30) - 1i64;
const UTIME_OMIT: i64 = (1i64 << 30) - 2i64;

/// A common function for updating file times, supporting both file descriptor (fd) and path-based
/// operations.
fn do_utimes(
    dirfd: RawFileDesc,
    pathname_ptr: Vaddr,
    times: Option<TimeSpecPair>,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = UtimensFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;

    // Unlike other system calls, `utimesat` has special handling for the NULL path string.
    // Reference: <https://elixir.bootlin.com/linux/v6.17.1/source/fs/utimes.c#L138-L139>
    let pathname = if dirfd != AT_FDCWD && pathname_ptr == 0 {
        None
    } else {
        let pathname = ctx
            .user_space()
            .read_cstring(pathname_ptr, MAX_FILENAME_LEN)?;
        Some(pathname.to_string_lossy().into_owned())
    };

    let path = if let Some(pathname) = pathname.as_ref() {
        let fs_path = FsPath::from_fd_at(dirfd, pathname, EmptyPathStr::AllowIfFlag(flags.bits()))?;
        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(UtimensFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    } else {
        // Linux will reject non-zero flags and `O_PATH` files if the pathname is null.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/fs/utimes.c#L108>
        if !flags.is_empty() {
            return_errno_with_message!(
                Errno::EINVAL,
                "flags must be zero when the pathname is NULL"
            );
        }
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, dirfd.try_into()?);
        if file.status_flags().contains(StatusFlags::O_PATH) {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        file.path().clone()
    };

    vfs_utimes(&path, times)
}

fn vfs_utimes(path: &Path, times: Option<TimeSpecPair>) -> Result<SyscallReturn> {
    let now = RealTimeCoarseClock::get().read_time();
    let (atime, mtime, ctime) = match times {
        Some(times) => {
            let atime = times.atime.with_now(now);
            let mtime = times.mtime.with_now(now);
            (atime, mtime, now)
        }
        None => (Some(now), Some(now), now),
    };

    // Update times.
    if let Some(atime) = atime {
        path.set_atime(atime);
    }
    if let Some(mtime) = mtime {
        path.set_mtime(mtime);
    }
    path.set_ctime(ctime);
    fs::vfs::notify::on_attr_change(path);
    Ok(SyscallReturn::Return(0))
}

bitflags::bitflags! {
    struct UtimensFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
