// SPDX-License-Identifier: MPL-2.0

//! The notification listener's file lifetime and attached-thread accounting.
//!
//! Notification state transitions follow Linux's seccomp notification protocol:
//! <https://github.com/torvalds/linux/blob/v6.18/kernel/seccomp.c>.

use core::{
    fmt::{self, Display},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

mod addfd;

use addfd::AddFdRequest;
use ostd::{
    mm::VmIo,
    sync::{WaitQueue, Waiter, Waker},
};
use spin::Once;

use super::abi::{SeccompData, SeccompNotif, SeccompNotifResp};
use crate::{
    context::current_userspace,
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{AccessMode, CreationFlags, FileCommon, FileLike, StatusFlags, file_table::FdFlags},
        pseudofs::AnonInodeFs,
        vfs::path::Path,
    },
    prelude::*,
    process::signal::{PauseReason, PollHandle, Pollable, Pollee},
    util::ioctl::RawIoctl,
};

static LISTENER_PATH: Once<Path> = Once::new();

pub(in crate::security) fn init() {
    // Share the pseudo dentry so listener installation needs no path allocation.
    LISTENER_PATH.call_once(|| AnonInodeFs::new_path(|_| "anon_inode:seccomp notify".to_string()));
}

/// Notification state shared by a filter and its listener file description.
pub(crate) struct Listener {
    is_open: AtomicBool,
    users: AtomicUsize,
    pollee: Pollee,
    notifications: Mutex<Notifications>,
    receivers: WaitQueue,
    wait_mode: NotificationWaitMode,
}

/// The signal behavior selected when a listener is installed.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum NotificationWaitMode {
    Interruptible,
    KillableAfterReceive,
}

#[derive(Default)]
struct Notifications {
    next_id: u64,
    // A canceled unread request still consumes one RECV attempt (ENOENT).
    pending_receives: usize,
    requests: Vec<Notification>,
}

impl Notifications {
    fn remove(&mut self, id: u64) -> Option<Notification> {
        let index = self
            .requests
            .iter()
            .position(|request| request.message.id == id)?;
        Some(self.requests.remove(index))
    }
}

struct Notification {
    message: SeccompNotif,
    state: NotificationState,
    add_fds: Vec<Arc<AddFdRequest>>,
    target: Arc<Waker>,
}

impl Notification {
    fn accepts_reply(&self) -> bool {
        matches!(self.state, NotificationState::Received)
            && !self.add_fds.last().is_some_and(|op| op.send)
    }
}

impl Drop for Notification {
    fn drop(&mut self) {
        for op in &self.add_fds {
            op.finish(Err(Error::new(Errno::ESRCH)));
        }
    }
}

enum NotificationState {
    Queued,
    Received,
    Replied(SeccompNotifResp),
}

enum WaitOutcome {
    Response(SeccompNotifResp),
    AddFd,
    ModeChanged,
}

const NOTIF_FLAG_CONTINUE: u32 = 1;

impl Listener {
    fn new(wait_mode: NotificationWaitMode) -> Result<Arc<Self>> {
        Arc::try_new(Self {
            is_open: AtomicBool::new(true),
            users: AtomicUsize::new(0),
            pollee: Pollee::try_new()?,
            notifications: Mutex::new(Notifications::default()),
            receivers: WaitQueue::new(),
            wait_mode,
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
            self.receivers.wake_all();
        }
    }

    /// Waits for a supervisor response, or returns `None` to continue the syscall.
    pub(crate) fn notify(&self, ctx: &Context, data: &SeccompData) -> Result<Option<usize>> {
        let (waiter, target) = Waiter::new_pair();
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
                add_fds: Vec::new(),
                target,
            });
            notifications.pending_receives = notifications.pending_receives.saturating_add(1);
            id
        };
        let _pending = PendingNotification { listener: self, id };
        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        self.receivers.wake_all();

        let response = self.wait_for_response(id, ctx, &waiter)?;
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

    fn wait_for_response(
        &self,
        id: u64,
        ctx: &Context,
        waiter: &Waiter,
    ) -> Result<SeccompNotifResp> {
        loop {
            let is_killable = self.should_sleep_killable(&self.notifications.lock(), id);
            let reason = if is_killable {
                PauseReason::Killable
            } else {
                PauseReason::Sleep
            };
            let outcome = waiter.pause_until_by(
                || {
                    let notifications = self.notifications.lock();
                    // The caller's PendingNotification owns this entry until
                    // the response or interruption has been consumed.
                    let request = notifications
                        .requests
                        .iter()
                        .find(|request| request.message.id == id)
                        .unwrap();
                    if !request.add_fds.is_empty() {
                        return Some(WaitOutcome::AddFd);
                    }
                    if let NotificationState::Replied(response) = request.state {
                        return Some(WaitOutcome::Response(response));
                    }
                    (self.should_sleep_killable(&notifications, id) != is_killable)
                        .then_some(WaitOutcome::ModeChanged)
                },
                reason,
            );
            match outcome {
                Ok(WaitOutcome::Response(response)) => return Ok(response),
                Ok(WaitOutcome::AddFd) => self.install_add_fd(id, ctx),
                Ok(WaitOutcome::ModeChanged) => continue,
                Err(_) => {
                    let mut notifications = self.notifications.lock();
                    // RECV can win a race with an ordinary signal. Linux
                    // switches to killable waiting rather than canceling the
                    // request in this case. A pending SIGKILL still interrupts
                    // the next killable wait.
                    if !is_killable && self.should_sleep_killable(&notifications, id) {
                        continue;
                    }
                    // Cancel under the same lock as the mode check so RECV
                    // cannot claim a request that we have decided to interrupt.
                    let canceled = notifications.remove(id);
                    drop(notifications);
                    drop(canceled);
                    return_errno_with_message!(
                        Errno::ERESTARTSYS,
                        "seccomp notification interrupted"
                    );
                }
            }
        }
    }

    fn should_sleep_killable(&self, notifications: &Notifications, id: u64) -> bool {
        self.wait_mode == NotificationWaitMode::KillableAfterReceive
            && notifications.requests.iter().any(|request| {
                request.message.id == id && !matches!(request.state, NotificationState::Queued)
            })
    }

    fn receive(&self) -> Result<SeccompNotif> {
        let result = self
            .receivers
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
        if !request.accepts_reply() {
            return;
        }
        request.state = NotificationState::Queued;
        request.target.wake_up();
        notifications.pending_receives = notifications.pending_receives.saturating_add(1);
        drop(notifications);
        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        self.receivers.wake_all();
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
        if !request.accepts_reply() {
            return_errno_with_message!(
                Errno::EINPROGRESS,
                "seccomp notification is not awaiting a reply"
            );
        }
        request.state = NotificationState::Replied(response);
        let target = request.target.clone();
        drop(notifications);
        self.pollee.invalidate();
        target.wake_up();
        Ok(())
    }

    fn validate_id(&self, id: u64) -> Result<()> {
        let notifications = self.notifications.lock();
        if notifications
            .requests
            .iter()
            .any(|request| request.message.id == id && request.accepts_reply())
        {
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
                request.target.wake_up();
            }
        }
        drop(notifications);
        self.pollee.invalidate();
        self.receivers.wake_all();
    }

    fn check_events(&self) -> IoEvents {
        let mut events = IoEvents::empty();
        for request in &self.notifications.lock().requests {
            match request.state {
                NotificationState::Queued => events |= IoEvents::IN | IoEvents::RDNORM,
                NotificationState::Received if request.accepts_reply() => {
                    events |= IoEvents::OUT | IoEvents::WRNORM
                }
                NotificationState::Received => {}
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
        let finished = notifications.remove(self.id);
        drop(notifications);
        drop(finished);
        self.listener.pollee.invalidate();
    }
}

/// An open seccomp notification listener, shared by dup and descriptor passing.
pub(super) struct ListenerFile {
    listener: Arc<Listener>,
    common: FileCommon,
}

impl ListenerFile {
    pub(super) fn new(wait_mode: NotificationWaitMode) -> Result<Arc<Self>> {
        let listener = Listener::new(wait_mode)?;
        let path = LISTENER_PATH.get().unwrap().clone();
        Arc::try_new(Self {
            listener,
            common: FileCommon::new(path, AccessMode::O_RDWR, StatusFlags::empty()),
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate a seccomp listener file"))
    }

    pub(super) fn listener(&self) -> &Arc<Listener> {
        &self.listener
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
            _ => return self.listener.add_fd(raw_ioctl),
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
