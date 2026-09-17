// SPDX-License-Identifier: MPL-2.0

use core::cmp::Ordering;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file::{
            FileLike, FileOwnerKind, FileOwnerTarget, StatusFlags, StatusFlagsUpdate,
            file_table::{FdFlags, FileDesc, FileTable, RawFileDesc, WithFileTable, get_file_fast},
        },
        ramfs::memfd::{FileSeals, MemfdInodeHandle},
        vfs::range_lock::{FileRange, OFFSET_MAX, RangeLockItem, RangeLockType},
    },
    prelude::*,
    process::{FileOwnerCreds, Pgid, Pid, pid_table},
};

pub(super) fn sys_fcntl(
    raw_fd: RawFileDesc,
    cmd: i32,
    arg: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let fd = FileDesc::try_from(raw_fd)?;
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);
    match fcntl_cmd {
        FcntlCmd::F_DUPFD => handle_dupfd(fd, arg, FdFlags::empty(), ctx),
        FcntlCmd::F_DUPFD_CLOEXEC => handle_dupfd(fd, arg, FdFlags::CLOEXEC, ctx),
        FcntlCmd::F_GETFD => handle_getfd(fd, ctx),
        FcntlCmd::F_SETFD => handle_setfd(fd, arg, ctx),
        FcntlCmd::F_GETFL => handle_getfl(fd, ctx),
        FcntlCmd::F_SETFL => handle_setfl(fd, arg, ctx),
        FcntlCmd::F_GETLK => handle_getlk(fd, arg, ctx),
        FcntlCmd::F_SETLK => handle_setlk(fd, arg, true, ctx),
        FcntlCmd::F_SETLKW => handle_setlk(fd, arg, false, ctx).map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        }),
        FcntlCmd::F_GETOWN => handle_getown(fd, ctx),
        FcntlCmd::F_SETOWN => handle_setown(fd, arg, ctx),
        FcntlCmd::F_GETOWN_EX => handle_getown_ex(fd, arg, ctx),
        FcntlCmd::F_SETOWN_EX => handle_setown_ex(fd, arg, ctx),
        FcntlCmd::F_ADD_SEALS => handle_addseal(fd, arg, ctx),
        FcntlCmd::F_GET_SEALS => handle_getseal(fd, ctx),
    }
}

fn handle_dupfd(fd: FileDesc, arg: u64, flags: FdFlags, ctx: &Context) -> Result<SyscallReturn> {
    let ceil_fd = (arg as RawFileDesc)
        .try_into()
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid fd"))?;

    let file_table = ctx.thread_local.borrow_file_table();
    let new_fd = file_table.unwrap().write().dup_ceil(fd, ceil_fd, flags)?;
    Ok(SyscallReturn::Return(new_fd.into()))
}

fn handle_getfd(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    file_table.read_with(|inner| {
        let fd_flags = inner.get_entry(fd)?.flags();
        Ok(SyscallReturn::Return(fd_flags.bits() as _))
    })
}

fn handle_setfd(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let flags = if arg > u64::from(u8::MAX) {
        return_errno_with_message!(Errno::EINVAL, "invalid fd flags");
    } else {
        FdFlags::from_bits(arg as u8)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid fd flags"))?
    };

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    file_table.read_with(|inner| {
        inner.get_entry(fd)?.set_flags(flags);
        Ok(SyscallReturn::Return(0))
    })
}

fn handle_getfl(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let status_flags = file.status_flags();
    let access_mode = file.access_mode();
    Ok(SyscallReturn::Return(
        (status_flags.bits() | access_mode as u32) as _,
    ))
}

fn handle_setfl(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let new_flags = StatusFlags::from_bits_truncate(arg as _);
    file.update_status_flags(StatusFlagsUpdate::replace(new_flags))?;

    Ok(SyscallReturn::Return(0))
}

fn handle_getlk(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let owner = FileTable::range_lock_owner(file_table.unwrap());
    let file = get_file_fast!(&mut file_table, fd);

    let lock_mut_ptr = arg as Vaddr;
    let mut lock_mut_c = ctx.user_space().read_val::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::try_from(lock_mut_c.l_type)?;
    if lock_type == RangeLockType::Unlock {
        return_errno_with_message!(Errno::EINVAL, "invalid flock type for getlk");
    }
    let lock = RangeLockItem::new(
        owner,
        ctx.process.pid(),
        lock_type,
        from_c_flock_and_file(&lock_mut_c, &**file)?,
    );

    let lock = file.as_inode_handle_or_err()?.test_range_lock(lock)?;

    lock_mut_c.copy_from_range_lock(&lock);
    ctx.user_space().write_val(lock_mut_ptr, &lock_mut_c)?;

    Ok(SyscallReturn::Return(0))
}

fn handle_setlk(
    fd: FileDesc,
    arg: u64,
    is_nonblocking: bool,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let owner = FileTable::range_lock_owner(file_table.unwrap());
    let file = get_file_fast!(&mut file_table, fd).into_owned();

    let lock_mut_ptr = arg as Vaddr;
    let lock_mut_c = ctx.user_space().read_val::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::try_from(lock_mut_c.l_type)?;
    let lock = RangeLockItem::new(
        owner,
        ctx.process.pid(),
        lock_type,
        from_c_flock_and_file(&lock_mut_c, &*file)?,
    );

    let inode_file = file.as_inode_handle_or_err()?;
    inode_file.set_range_lock(&lock, is_nonblocking)?;

    if lock.type_() == RangeLockType::Unlock {
        return Ok(SyscallReturn::Return(0));
    }
    // A concurrent close will release the range locks for this owner and inode
    // but may miss the new one. If it happens, release the new lock to prevent
    // it from leaking forever.
    let file_is_still_open = file_table.read_with(|table| {
        table
            .get_file(fd)
            .is_ok_and(|current_file| Arc::ptr_eq(current_file, &file))
    });
    if !file_is_still_open {
        inode_file.release_range_locks(owner);
        return_errno_with_message!(
            Errno::EBADF,
            "the file descriptor was closed while setting a range lock"
        );
    }

    Ok(SyscallReturn::Return(0))
}

fn handle_getown(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    // A process ID is returned as a positive value; a process group ID is returned as a negative
    // value. Like Linux, we do not special-case the process group IDs that a libc wrapper may
    // mistake for error codes; `F_GETOWN_EX` is the way to avoid that ambiguity.
    let id = file.common().owner().id().unwrap_or(0);
    Ok(SyscallReturn::Return(id as _))
}

fn handle_setown(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    // A process ID is specified as a positive value; a process group ID is specified as a negative
    // value. Zero clears the owner.
    let who = arg as i32;
    // `i32::MIN` has no positive counterpart, so reject it before negating it below.
    if who == i32::MIN {
        return_errno_with_message!(Errno::EINVAL, "the file owner ID is out of range");
    }

    // `f_setown` records `PIDTYPE_TGID` for a cleared owner as well as a positive one, so
    // only a negative argument selects the process-group kind.
    let kind = if who < 0 {
        FileOwnerKind::ProcessGroup
    } else {
        FileOwnerKind::Process
    };

    let owner = match who.cmp(&0) {
        Ordering::Equal => None,
        Ordering::Greater => {
            let pid = who as Pid;
            let process = pid_table::pid_table_mut().get_process(pid).ok_or_else(|| {
                Error::with_message(
                    Errno::ESRCH,
                    "the process to be a file owner does not exist",
                )
            })?;
            Some(FileOwnerTarget::Process(process))
        }
        Ordering::Less => {
            let pgid: Pgid = who.unsigned_abs();
            let group = pid_table::pid_table_mut()
                .get_process_group(&pgid)
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::ESRCH,
                        "the process group to be a file owner does not exist",
                    )
                })?;
            Some(FileOwnerTarget::ProcessGroup(group))
        }
    };

    // Record the caller's credentials now. Linux checks these saved values when the signal
    // is later delivered, not the credentials of whoever is running at that point.
    let creds = FileOwnerCreds::new_from(&ctx.posix_thread.credentials());

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    file.set_owner(owner.as_ref(), kind, creds);

    Ok(SyscallReturn::Return(0))
}

/// C struct `f_owner_ex`, the argument to `F_SETOWN_EX` and `F_GETOWN_EX`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct c_f_owner_ex {
    /// One of the `F_OWNER_*` constants, i.e. a [`FileOwnerKind`].
    type_: i32,
    /// The thread, process or process group ID, always as a positive value.
    pid: i32,
}

fn handle_getown_ex(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let (kind, id) = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd);
        let owner = file.common().owner();
        (owner.kind(), owner.id())
    };

    // With no owner at all Linux still reports a type, and the type it reports is
    // `F_OWNER_TID`. The identifier is also zeroed once the owner has exited, while the
    // recorded type keeps being reported.
    let owner_ex = c_f_owner_ex {
        type_: kind.unwrap_or(FileOwnerKind::Thread) as i32,
        // `F_GETOWN` reports a process group negated, but `f_owner_ex` does not: the type
        // field already says which kind of ID this is.
        pid: id.unwrap_or(0).abs(),
    };

    ctx.user_space().write_val(arg as Vaddr, &owner_ex)?;

    Ok(SyscallReturn::Return(0))
}

fn handle_setown_ex(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let owner_ex: c_f_owner_ex = ctx.user_space().read_val(arg as Vaddr)?;

    let kind = FileOwnerKind::try_from(owner_ex.type_)
        .map_err(|_| Error::with_message(Errno::EINVAL, "the file owner type is not valid"))?;

    // Unlike `F_SETOWN`, the ID is not negated to select a process group here: the type
    // field carries what `F_SETOWN` encoded in the sign. Zero clears the owner, and a
    // negative value simply names nothing, which Linux reports as `ESRCH` rather than
    // rejecting it as malformed.
    let owner = match owner_ex.pid {
        0 => None,
        pid if pid < 0 => {
            return_errno_with_message!(
                Errno::ESRCH,
                "the thread, process or process group to be a file owner does not exist"
            )
        }
        pid => Some(lookup_owner_target(kind, pid as u32)?),
    };

    let creds = FileOwnerCreds::new_from(&ctx.posix_thread.credentials());

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    file.set_owner(owner.as_ref(), kind, creds);

    Ok(SyscallReturn::Return(0))
}

/// Resolves a `F_SETOWN_EX` identifier of the given kind into an owner target.
fn lookup_owner_target(kind: FileOwnerKind, id: u32) -> Result<FileOwnerTarget> {
    match kind {
        FileOwnerKind::Thread => {
            let thread = pid_table::pid_table_mut().get_thread(id).ok_or_else(|| {
                Error::with_message(Errno::ESRCH, "the thread to be a file owner does not exist")
            })?;
            Ok(FileOwnerTarget::Thread(thread))
        }
        FileOwnerKind::Process => {
            let process = pid_table::pid_table_mut().get_process(id).ok_or_else(|| {
                Error::with_message(
                    Errno::ESRCH,
                    "the process to be a file owner does not exist",
                )
            })?;
            Ok(FileOwnerTarget::Process(process))
        }
        FileOwnerKind::ProcessGroup => {
            let group = pid_table::pid_table_mut()
                .get_process_group(&id)
                .ok_or_else(|| {
                    Error::with_message(
                        Errno::ESRCH,
                        "the process group to be a file owner does not exist",
                    )
                })?;
            Ok(FileOwnerTarget::ProcessGroup(group))
        }
    }
}

fn handle_addseal(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let new_seals = FileSeals::from_bits(arg as u32)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid seals"))?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    file.as_inode_handle_or_err()?.add_seals(new_seals)?;

    Ok(SyscallReturn::Return(0))
}

fn handle_getseal(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let file_seals = file.as_inode_handle_or_err()?.get_seals()?;
    Ok(SyscallReturn::Return(file_seals.bits() as _))
}

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum FcntlCmd {
    F_DUPFD = 0,
    F_GETFD = 1,
    F_SETFD = 2,
    F_GETFL = 3,
    F_SETFL = 4,
    F_GETLK = 5,
    F_SETLK = 6,
    F_SETLKW = 7,
    F_SETOWN = 8,
    F_GETOWN = 9,
    F_SETOWN_EX = 15,
    F_GETOWN_EX = 16,
    F_DUPFD_CLOEXEC = 1030,
    F_ADD_SEALS = 1033,
    F_GET_SEALS = 1034,
}

#[expect(non_camel_case_types)]
type off_t = i64;

#[expect(non_camel_case_types)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum RangeLockWhence {
    SEEK_SET = 0,
    SEEK_CUR = 1,
    SEEK_END = 2,
}

/// C struct for a file range lock in Libc
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct c_flock {
    /// Type of lock: F_RDLCK, F_WRLCK, or F_UNLCK
    pub l_type: u16,
    /// Where `l_start' is relative to
    pub l_whence: u16,
    /// Offset where the lock begins
    pub l_start: off_t,
    /// Size of the locked area, 0 means until EOF
    pub l_len: off_t,
    /// Process holding the lock
    pub l_pid: Pid,
}

impl c_flock {
    pub(crate) fn copy_from_range_lock(&mut self, lock: &RangeLockItem) {
        self.l_type = lock.type_() as u16;
        if RangeLockType::Unlock != lock.type_() {
            self.l_whence = RangeLockWhence::SEEK_SET as u16;
            self.l_start = lock.start() as off_t;
            self.l_len = if lock.end() == OFFSET_MAX {
                0
            } else {
                lock.range().len() as off_t
            };
            self.l_pid = lock.pid();
        }
    }
}

/// Create the file range through C flock and opened file reference
fn from_c_flock_and_file(lock: &c_flock, file: &dyn FileLike) -> Result<FileRange> {
    let start = {
        let whence = RangeLockWhence::try_from(lock.l_whence)?;
        match whence {
            RangeLockWhence::SEEK_SET => lock.l_start,
            RangeLockWhence::SEEK_CUR => (file.as_inode_handle_or_err()?.offset() as off_t)
                .checked_add(lock.l_start)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "start overflow"))?,

            RangeLockWhence::SEEK_END => (file.path().inode().metadata()?.size as off_t)
                .checked_add(lock.l_start)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "start overflow"))?,
        }
    };

    if start < 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid start");
    }

    let (start, end) = match lock.l_len {
        len if len > 0 => {
            let end = start
                .checked_add(len)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "end overflow"))?;
            (start as usize, end as usize)
        }
        0 => (start as usize, OFFSET_MAX),
        len if len < 0 => {
            let end = start;
            // `start + len` won't overflow because `start >= 0` and `len < 0`.
            let new_start = start + len;
            if new_start < 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid len");
            }
            (new_start as usize, end as usize)
        }
        _ => unreachable!(),
    };

    FileRange::new(start, end)
}
