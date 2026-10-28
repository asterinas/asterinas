// SPDX-License-Identifier: MPL-2.0

//! The notification listener's file lifetime and attached-thread accounting.
//!
//! Notification state transitions follow Linux's seccomp notification protocol:
//! <https://github.com/torvalds/linux/blob/v6.18/kernel/seccomp.c>.

use core::{
    fmt::{self, Display},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ostd::{mm::VmIo, sync::WaitQueue};

use super::abi::{SeccompData, SeccompNotif, SeccompNotifResp};
use crate::{
    context::current_userspace,
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::ioctl::RawIoctl,
};

/// Notification state shared by a filter and its listener file description.
pub(crate) struct Listener {
    is_open: AtomicBool,
    users: AtomicUsize,
    pollee: Pollee,
    notifications: Mutex<Notifications>,
    waiters: WaitQueue,
}

#[derive(Default)]
struct Notifications {
    next_id: u64,
    // A canceled unread request still consumes one RECV attempt (ENOENT).
    pending_receives: usize,
    requests: Vec<Notification>,
}

struct Notification {
    message: SeccompNotif,
    state: NotificationState,
}

enum NotificationState {
    Queued,
    Received,
    Replied(SeccompNotifResp),
}

const NOTIF_FLAG_CONTINUE: u32 = 1;

impl Listener {
    pub(super) fn new() -> Result<Arc<Self>> {
        Arc::try_new(Self {
            is_open: AtomicBool::new(true),
            users: AtomicUsize::new(0),
            pollee: Pollee::new(),
            notifications: Mutex::new(Notifications::default()),
            waiters: WaitQueue::new(),
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp listener"))
    }

    pub(super) fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Acquire)
    }

    pub(super) fn attach_thread(&self) {
        // Only attached threads count; temporary policy snapshots and listener
        // files cannot prevent POLLHUP. The count is bounded by the TID space.
        self.users.fetch_add(1, Ordering::Relaxed);
        self.pollee.invalidate();
    }

    pub(super) fn detach_thread(&self) {
        let previous = self.users.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
        if previous == 1 {
            self.pollee.notify(IoEvents::HUP);
            self.waiters.wake_all();
        }
    }

    /// Waits for a supervisor response, or returns `None` to continue the syscall.
    pub(crate) fn notify(&self, ctx: &Context, data: &SeccompData) -> Result<Option<usize>> {
        let id = {
            let mut notifications = self.notifications.lock();
            if !self.is_open() {
                return_errno_with_message!(Errno::ENOSYS, "no seccomp listener");
            }
            let next_id = notifications.next_id.checked_add(1).ok_or_else(|| {
                Error::with_message(Errno::EOVERFLOW, "seccomp notification IDs exhausted")
            })?;
            notifications.requests.try_reserve(1).map_err(|_| {
                Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp notification")
            })?;
            let id = notifications.next_id;
            notifications.next_id = next_id;
            notifications.requests.push(Notification {
                message: SeccompNotif {
                    id,
                    // The kernel currently supports only the initial PID namespace.
                    pid: ctx.posix_thread.tid(),
                    flags: 0,
                    data: *data,
                },
                state: NotificationState::Queued,
            });
            notifications.pending_receives = notifications.pending_receives.saturating_add(1);
            id
        };
        let _pending = PendingNotification { listener: self, id };
        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        self.waiters.wake_all();

        let response = self
            .waiters
            .pause_until(|| {
                let notifications = self.notifications.lock();
                let request = notifications
                    .requests
                    .iter()
                    .find(|request| request.message.id == id)?;
                match request.state {
                    NotificationState::Replied(response) => Some(response),
                    _ => None,
                }
            })
            .map_err(|_| {
                Error::with_message(Errno::ERESTARTSYS, "seccomp notification interrupted")
            })?;
        if response.flags & NOTIF_FLAG_CONTINUE != 0 {
            return Ok(None);
        }
        // An arbitrary supervisor-provided error is a register value, not a
        // kernel Error: in particular, -ERESTARTSYS must not request a restart.
        let value = if response.error != 0 {
            response.error as isize as usize
        } else {
            response.val as usize
        };
        Ok(Some(value))
    }

    fn receive(&self) -> Result<SeccompNotif> {
        let result = self
            .waiters
            .pause_until(|| {
                let mut notifications = self.notifications.lock();
                if notifications.pending_receives == 0 {
                    return (self.users.load(Ordering::Acquire) == 0).then(|| {
                        Err(Error::with_message(
                            Errno::ENOENT,
                            "no seccomp notification",
                        ))
                    });
                }
                notifications.pending_receives -= 1;
                let request = notifications
                    .requests
                    .iter_mut()
                    .find(|request| matches!(request.state, NotificationState::Queued));
                Some(match request {
                    Some(request) => {
                        request.state = NotificationState::Received;
                        Ok(request.message)
                    }
                    None => Err(Error::with_message(
                        Errno::ENOENT,
                        "seccomp notification canceled",
                    )),
                })
            })
            .map_err(|_| Error::with_message(Errno::ERESTARTSYS, "seccomp receive interrupted"))?;
        if result.is_ok() {
            self.pollee.notify(IoEvents::OUT | IoEvents::WRNORM);
        }
        result
    }

    fn requeue(&self, id: u64) {
        let mut notifications = self.notifications.lock();
        let Some(request) = notifications
            .requests
            .iter_mut()
            .find(|request| request.message.id == id)
        else {
            return;
        };
        request.state = NotificationState::Queued;
        notifications.pending_receives = notifications.pending_receives.saturating_add(1);
        drop(notifications);
        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        self.waiters.wake_all();
    }

    fn respond(&self, response: SeccompNotifResp) -> Result<()> {
        if response.flags & !NOTIF_FLAG_CONTINUE != 0
            || (response.flags & NOTIF_FLAG_CONTINUE != 0
                && (response.val != 0 || response.error != 0))
        {
            return_errno_with_message!(Errno::EINVAL, "invalid seccomp notification response");
        }
        let mut notifications = self.notifications.lock();
        let request = notifications
            .requests
            .iter_mut()
            .find(|request| request.message.id == response.id)
            .ok_or_else(|| {
                Error::with_message(Errno::ENOENT, "seccomp notification does not exist")
            })?;
        if !matches!(request.state, NotificationState::Received) {
            return_errno_with_message!(
                Errno::EINPROGRESS,
                "seccomp notification is not awaiting a reply"
            );
        }
        request.state = NotificationState::Replied(response);
        drop(notifications);
        self.pollee.invalidate();
        self.waiters.wake_all();
        Ok(())
    }

    fn validate_id(&self, id: u64) -> Result<()> {
        let notifications = self.notifications.lock();
        if notifications.requests.iter().any(|request| {
            request.message.id == id && matches!(request.state, NotificationState::Received)
        }) {
            Ok(())
        } else {
            return_errno_with_message!(
                Errno::ENOENT,
                "seccomp notification is not awaiting a reply"
            )
        }
    }

    fn close(&self) {
        let mut notifications = self.notifications.lock();
        // Serialize closure with enqueueing, so every enqueued task is released.
        self.is_open.store(false, Ordering::Release);
        for request in &mut notifications.requests {
            if !matches!(request.state, NotificationState::Replied(_)) {
                request.state = NotificationState::Replied(SeccompNotifResp {
                    id: request.message.id,
                    val: 0,
                    error: -(Errno::ENOSYS as i32),
                    flags: 0,
                });
            }
        }
        drop(notifications);
        self.pollee.invalidate();
        self.waiters.wake_all();
    }

    fn check_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();
        for request in &self.notifications.lock().requests {
            match request.state {
                NotificationState::Queued => events |= IoEvents::IN | IoEvents::RDNORM,
                NotificationState::Received => events |= IoEvents::OUT | IoEvents::WRNORM,
                NotificationState::Replied(_) => {}
            }
        }
        if self.users.load(Ordering::Acquire) == 0 {
            events |= IoEvents::HUP;
        }
        events
    }
}

/// Removes a request on both normal completion and signal cancellation.
struct PendingNotification<'a> {
    listener: &'a Listener,
    id: u64,
}

impl Drop for PendingNotification<'_> {
    fn drop(&mut self) {
        let mut notifications = self.listener.notifications.lock();
        if let Some(index) = notifications
            .requests
            .iter()
            .position(|request| request.message.id == self.id)
        {
            notifications.requests.remove(index);
        }
        drop(notifications);
        self.listener.pollee.invalidate();
    }
}

/// An open seccomp notification listener, shared by dup and descriptor passing.
pub(super) struct ListenerFile {
    listener: Arc<Listener>,
    common: FileCommon,
}

impl ListenerFile {
    pub(super) fn new(listener: Arc<Listener>) -> Result<Arc<Self>> {
        let path = AnonInodeFs::new_path(|_| "anon_inode:seccomp notify".to_string());
        Arc::try_new(Self {
            listener,
            common: FileCommon::new(path, AccessMode::O_RDWR, StatusFlags::empty()),
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp listener file"))
    }
}

impl Drop for ListenerFile {
    fn drop(&mut self) {
        self.listener.close();
    }
}

impl Pollable for ListenerFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.listener.poll(mask, poller)
    }
}

impl Pollable for Listener {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || self.check_events())
    }
}

impl FileLike for ListenerFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ Receive => {
                cmd.with_data_ptr(|ptr| {
                    // Like Linux check_zeroed_user(), stop at the first nonzero
                    // byte rather than faulting on a later inaccessible page.
                    let mut reader = ptr
                        .vm()
                        .reader(raw_ioctl.arg(), size_of::<SeccompNotif>())?;
                    while reader.has_remain() {
                        if reader.read_val::<u8>()? != 0 {
                            return_errno_with_message!(
                                Errno::EINVAL,
                                "seccomp receive buffer must be zeroed"
                            );
                        }
                    }
                    Ok(())
                })?;
                let notification = self.listener.receive()?;
                if let Err(error) = cmd.write(&notification) {
                    self.listener.requeue(notification.id);
                    return Err(error);
                }
            }
            cmd @ Respond => {
                self.listener.respond(cmd.read()?)?;
            }
            cmd @ ValidateId => {
                self.listener.validate_id(cmd.read()?)?;
            }
            ValidateIdLegacy => {
                // The original ID_VALID ioctl accidentally specified _IOR,
                // but its argument is still read from userspace.
                let id = current_userspace!().read_val(raw_ioctl.arg())?;
                self.listener.validate_id(id)?;
            }
            _ =>
                return_errno_with_message!(Errno::EINVAL, "unsupported seccomp notification ioctl"),
        });
        Ok(0)
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            flags: u32,
        }
        impl Display for FdInfo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                writeln!(f, "pos:\t0")?;
                writeln!(f, "flags:\t0{:o}", self.flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())
            }
        }
        let mut flags = self.common.status_flags().bits() | self.common.access_mode() as u32;
        if fd_flags.contains(FdFlags::CLOEXEC) {
            flags |= CreationFlags::O_CLOEXEC.bits();
        }
        Box::new(FdInfo { flags })
    }
}

mod ioctl_defs {
    use super::{SeccompNotif, SeccompNotifResp};
    use crate::{
        ioc,
        util::ioctl::{InData, InOutData, OutData},
    };

    // Reference: <https://github.com/torvalds/linux/blob/v6.18/include/uapi/linux/seccomp.h>.
    pub(super) type Receive = ioc!(SECCOMP_IOCTL_NOTIF_RECV, b'!', 0, InOutData<SeccompNotif>);
    pub(super) type Respond = ioc!(SECCOMP_IOCTL_NOTIF_SEND, b'!', 1, InOutData<SeccompNotifResp>);
    pub(super) type ValidateId = ioc!(SECCOMP_IOCTL_NOTIF_ID_VALID, b'!', 2, InData<u64>);
    pub(super) type ValidateIdLegacy = ioc!(SECCOMP_IOCTL_NOTIF_ID_VALID_WRONG_DIR, b'!', 2, OutData<u64>);
}
