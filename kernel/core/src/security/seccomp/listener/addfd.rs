// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::{Listener, NotificationState, SeccompNotifResp, WaitQueue};
use crate::{
    events::IoEvents,
    fs::file::{
        CreationFlags, FileLike,
        file_table::{ClosedFile, FdFlags, FileDesc},
    },
    prelude::*,
    process::ResourceType,
    security::seccomp::abi::SeccompNotifAddFd,
    util::ioctl::RawIoctl,
};

const ADDFD_FLAG_SETFD: u32 = 1;
const ADDFD_FLAG_SEND: u32 = 2;

/// The supervisor holds its reference until the target completes the transfer
/// or the supervisor removes the request under the notification lock.
pub(super) struct AddFdRequest {
    file: Arc<dyn FileLike>,
    dest: Option<FileDesc>,
    fd_flags: FdFlags,
    pub(super) send: bool,
    result: SpinLock<Option<Result<FileDesc>>>,
    completion: WaitQueue,
}

impl AddFdRequest {
    pub(super) fn finish(&self, result: Result<FileDesc>) {
        *self.result.lock() = Some(result);
        self.completion.wake_all();
    }

    fn install(&self, ctx: &Context) -> Result<(FileDesc, Option<ClosedFile>)> {
        let table = ctx.thread_local.borrow_file_table();
        let mut table = table.write();
        if let Some(fd) = self.dest {
            let limit = ctx
                .process
                .resource_limits()
                .get_rlimit(ResourceType::RLIMIT_NOFILE)
                .get_cur();
            if u64::from(fd) >= limit {
                return_errno_with_message!(Errno::EBADF, "the new FD reaches the FD limit");
            }
            Ok((fd, table.replace_file(fd, &self.file, self.fd_flags)?))
        } else {
            let fd = table
                .insert(self.file.clone(), self.fd_flags)
                .map_err(|(err, _)| err)?;
            Ok((fd, None))
        }
    }
}

impl Listener {
    pub(super) fn add_fd(&self, ioctl: RawIoctl) -> Result<i32> {
        // Linux dispatches extensible ioctls by type and number, ignoring
        // direction and decoding the size from bits 16..29.
        if ioctl.cmd() as u16 != u16::from_be_bytes([b'!', 3]) {
            return_errno_with_message!(Errno::EINVAL, "unsupported seccomp notification ioctl");
        }
        let size = ((ioctl.cmd() >> 16) & 0x3fff) as usize;
        if !(size_of::<SeccompNotifAddFd>()..PAGE_SIZE).contains(&size) {
            return_errno_with_message!(Errno::EINVAL, "invalid seccomp addfd size");
        }
        let args = {
            let current = Task::current().unwrap();
            let user = CurrentUserSpace::new(current.as_thread_local().unwrap());
            let mut reader = user.reader(ioctl.arg(), size)?;
            let args = reader.read_val::<SeccompNotifAddFd>()?;
            while reader.has_remain() {
                if reader.read_val::<u8>()? != 0 {
                    return_errno_with_message!(Errno::E2BIG, "unknown seccomp addfd fields");
                }
            }
            args
        };
        if args.flags & !(ADDFD_FLAG_SETFD | ADDFD_FLAG_SEND) != 0
            || args.newfd_flags & !CreationFlags::O_CLOEXEC.bits() != 0
            || (args.flags & ADDFD_FLAG_SETFD == 0 && args.newfd != 0)
        {
            return_errno_with_message!(Errno::EINVAL, "invalid seccomp addfd arguments");
        }
        let file = {
            let current = Task::current().unwrap();
            let table = current.as_thread_local().unwrap().borrow_file_table();
            table
                .read()
                .get_file((args.srcfd as i32).try_into()?)?
                .clone()
        };
        let op = Arc::try_new(AddFdRequest {
            file,
            dest: (args.flags & ADDFD_FLAG_SETFD != 0)
                .then(|| (args.newfd as i32).try_into())
                .transpose()?,
            fd_flags: if args.newfd_flags == 0 {
                FdFlags::empty()
            } else {
                FdFlags::CLOEXEC
            },
            send: args.flags & ADDFD_FLAG_SEND != 0,
            result: SpinLock::new(None),
            completion: WaitQueue::new(),
        })
        .map_err(|_| Error::new(Errno::ENOMEM))?;
        let target = {
            let mut notifications = self.notifications.lock();
            let request = notifications
                .requests
                .iter_mut()
                .find(|n| n.message.id == args.id)
                .ok_or_else(|| Error::new(Errno::ENOENT))?;
            if !request.accepts_reply() {
                return_errno_with_message!(
                    Errno::EINPROGRESS,
                    "notification is not awaiting a reply"
                );
            }
            if op.send && !request.add_fds.is_empty() {
                return_errno_with_message!(Errno::EBUSY, "other addfd requests are pending");
            }
            request
                .add_fds
                .try_reserve(1)
                .map_err(|_| Error::new(Errno::ENOMEM))?;
            request.add_fds.push(op.clone());
            request.target.clone()
        };
        if op.send {
            self.pollee.invalidate();
        }
        target.wake_up();
        match op.completion.pause_until(|| *op.result.lock()) {
            Ok(result) => result.map(Into::into),
            Err(err) => {
                let mut notifications = self.notifications.lock();
                if let Some(result) = *op.result.lock() {
                    return result.map(Into::into);
                }
                if let Some(request) = notifications
                    .requests
                    .iter_mut()
                    .find(|n| n.message.id == args.id)
                {
                    request.add_fds.retain(|queued| !Arc::ptr_eq(&op, queued));
                }
                drop(notifications);
                if op.send {
                    self.pollee.notify(IoEvents::OUT | IoEvents::WRNORM);
                }
                Err(err)
            }
        }
    }

    /// Runs in the notifying thread so allocation uses its current file table
    /// and resource limit. SEND commits the descriptor and reply together.
    pub(super) fn install_add_fd(&self, id: u64, ctx: &Context) {
        let mut notifications = self.notifications.lock();
        let request = notifications
            .requests
            .iter_mut()
            .find(|n| n.message.id == id)
            .unwrap();
        let Some(op) = request.add_fds.pop() else {
            // The supervisor was interrupted before we acquired the lock.
            return;
        };
        let mut displaced = None;
        let result = op.install(ctx).map(|(fd, closed)| {
            displaced = closed;
            if op.send {
                request.state = NotificationState::Replied(SeccompNotifResp {
                    id,
                    val: i64::from(i32::from(fd)),
                    error: 0,
                    flags: 0,
                });
            }
            fd
        });
        op.finish(result);
        drop(notifications);
        if op.send && result.is_err() {
            self.pollee.notify(IoEvents::OUT | IoEvents::WRNORM);
        }
        // Closing the displaced file may acquire a listener lock.
        drop(displaced);
    }
}
