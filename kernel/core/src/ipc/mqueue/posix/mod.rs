// SPDX-License-Identifier: MPL-2.0

//! POSIX message queue.

mod queue;

use core::{
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};

use ostd::mm::VmIo;
pub(crate) use queue::{MqAttr, MqueueQueue};
use spin::Once;

use crate::{
    fs::file::InodeMode,
    prelude::*,
    process::{
        Gid, Uid, UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread,
    },
    security::lsm::hooks as lsm_hooks,
    time::{clocks::RealTimeClock, timer::Timeout, timespec_t, wait::ManagedTimeout},
};

/// Maximum length of a queue name, excluding the leading `/`.
pub(crate) const NAME_MAX: usize = 255;
/// Hard upper bound on `mq_maxmsg`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/ipc/mqueue.c#L26>
pub(crate) const HARD_MSGMAX: i64 = 16384;
/// Hard upper bound on `mq_msgsize`.
pub(crate) const HARD_MSGSIZEMAX: i64 = 1048576;
/// Default `mq_maxmsg` when `mq_open` is called with a null `struct mq_attr`.
///
/// Reference: `/proc/sys/fs/mqueue/msg_default`, which defaults to 10.
pub(crate) const DFLT_MSGMAX: i64 = 10;
/// Default `mq_msgsize` when `mq_open` is called with a null `struct mq_attr`.
///
/// Reference: `/proc/sys/fs/mqueue/msgsize_default`, which defaults to 8192.
pub(crate) const DFLT_MSGSIZEMAX: i64 = 8192;

/// Default value of `/proc/sys/fs/mqueue/queues_max`.
const DFLT_QUEUES_MAX: i64 = 256;
/// Default value of `/proc/sys/fs/mqueue/msg_max`.
const DFLT_MSG_MAX: i64 = 10;
/// Default value of `/proc/sys/fs/mqueue/msgsize_max`.
const DFLT_MSGSIZE_MAX: i64 = 8192;

static QUEUES_MAX: AtomicI64 = AtomicI64::new(DFLT_QUEUES_MAX);
static MSG_MAX: AtomicI64 = AtomicI64::new(DFLT_MSG_MAX);
static MSG_DEFAULT: AtomicI64 = AtomicI64::new(DFLT_MSGMAX);
static MSGSIZE_MAX: AtomicI64 = AtomicI64::new(DFLT_MSGSIZE_MAX);
static MSGSIZE_DEFAULT: AtomicI64 = AtomicI64::new(DFLT_MSGSIZEMAX);

/// Returns the value of `/proc/sys/fs/mqueue/queues_max`.
pub(crate) fn queues_max() -> i64 {
    QUEUES_MAX.load(Ordering::Relaxed)
}

/// Returns the value of `/proc/sys/fs/mqueue/msg_max`.
pub(crate) fn msg_max() -> i64 {
    MSG_MAX.load(Ordering::Relaxed)
}

/// Returns the value of `/proc/sys/fs/mqueue/msg_default`.
pub(crate) fn msg_default() -> i64 {
    MSG_DEFAULT.load(Ordering::Relaxed)
}

/// Returns the value of `/proc/sys/fs/mqueue/msgsize_max`.
pub(crate) fn msgsize_max() -> i64 {
    MSGSIZE_MAX.load(Ordering::Relaxed)
}

/// Returns the value of `/proc/sys/fs/mqueue/msgsize_default`.
pub(crate) fn msgsize_default() -> i64 {
    MSGSIZE_DEFAULT.load(Ordering::Relaxed)
}

/// Sets the value of `/proc/sys/fs/mqueue/queues_max`.
pub(crate) fn set_queues_max(value: i64) -> Result<()> {
    store_sysctl(&QUEUES_MAX, value)
}

/// Sets the value of `/proc/sys/fs/mqueue/msg_max`.
pub(crate) fn set_msg_max(value: i64) -> Result<()> {
    store_sysctl(&MSG_MAX, value)
}

/// Sets the value of `/proc/sys/fs/mqueue/msg_default`.
pub(crate) fn set_msg_default(value: i64) -> Result<()> {
    store_sysctl(&MSG_DEFAULT, value)
}

/// Sets the value of `/proc/sys/fs/mqueue/msgsize_max`.
pub(crate) fn set_msgsize_max(value: i64) -> Result<()> {
    store_sysctl(&MSGSIZE_MAX, value)
}

/// Sets the value of `/proc/sys/fs/mqueue/msgsize_default`.
pub(crate) fn set_msgsize_default(value: i64) -> Result<()> {
    store_sysctl(&MSGSIZE_DEFAULT, value)
}

fn store_sysctl(sysctl: &AtomicI64, value: i64) -> Result<()> {
    if value < 0 {
        return_errno_with_message!(Errno::EINVAL, "the sysctl value cannot be negative");
    }
    sysctl.store(value, Ordering::Relaxed);
    Ok(())
}

/// Validates a queue name.
///
/// POSIX names look like `"/name"`, but glibc and musl strip the leading `/`
/// before issuing the system call, so the kernel only sees `name`.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return_errno_with_message!(Errno::EINVAL, "the queue name cannot be empty");
    }
    if name.len() > NAME_MAX {
        return_errno_with_message!(Errno::ENAMETOOLONG, "the queue name is too long");
    }
    if name.contains('/') {
        return_errno_with_message!(Errno::EINVAL, "the queue name cannot contain '/'");
    }
    Ok(())
}

/// The global registry of live POSIX message queues.
fn queue_table() -> &'static Mutex<BTreeMap<String, Arc<MqueueQueue>>> {
    static TABLE: Once<Mutex<BTreeMap<String, Arc<MqueueQueue>>>> = Once::new();
    TABLE.call_once(|| Mutex::new(BTreeMap::new()))
}

/// Looks up an existing queue by name.
pub(crate) fn lookup_queue(name: &str) -> Result<Arc<MqueueQueue>> {
    validate_name(name)?;

    let table = queue_table().lock();
    table
        .get(name)
        .cloned()
        .ok_or_else(|| Error::new(Errno::ENOENT))
}

/// Removes a queue by name.
///
/// The `mqueuefs` root directory is sticky, so only the owner of the queue, the
/// owner of the root directory, or a thread with `CAP_FOWNER` may unlink a
/// queue. Linux reports `EACCES` when this check fails.
///
/// Queues that are still open keep their state alive until the last file
/// descriptor is closed, matching the Linux behavior of `mq_unlink`.
pub(crate) fn unlink_queue(name: &str) -> Result<()> {
    validate_name(name)?;

    // Query the credentials and the capability before taking the lock, since
    // the LSM hooks may acquire other locks.
    let fsuid = current_fsuid();
    let is_privileged = fsuid.is_root() || has_cap_fowner();

    let mut table = queue_table().lock();
    let Some(queue) = table.get(name) else {
        return_errno_with_message!(Errno::ENOENT, "the message queue does not exist");
    };
    if !is_privileged && queue.owner() != fsuid {
        return_errno_with_message!(
            Errno::EACCES,
            "only the owner of the message queue may unlink it"
        );
    }
    table.remove(name);
    Ok(())
}

/// Creates a new queue with the given permission and attributes and registers it.
pub(crate) fn create_queue(name: &str, mode: InodeMode, attr: MqAttr) -> Result<Arc<MqueueQueue>> {
    validate_name(name)?;

    let is_privileged = has_cap_sys_resource();

    let mut table = queue_table().lock();
    if table.contains_key(name) {
        return_errno!(Errno::EEXIST);
    }

    if !is_privileged && table.len() as i64 >= queues_max() {
        return_errno_with_message!(Errno::ENOSPC, "the message queue limit has been reached");
    }

    validate_attr(attr, is_privileged)?;

    let queue = Arc::new(MqueueQueue::new(
        mode,
        current_fsuid(),
        current_fsgid(),
        attr.mq_maxmsg,
        attr.mq_msgsize,
    ));
    table.insert(name.to_string(), queue.clone());
    Ok(queue)
}

/// Validates the attributes used to create a new queue.
fn validate_attr(attr: MqAttr, is_privileged: bool) -> Result<()> {
    if attr.mq_maxmsg <= 0 || attr.mq_msgsize <= 0 {
        return_errno_with_message!(Errno::EINVAL, "mq_maxmsg and mq_msgsize must be positive");
    }

    // Unprivileged callers are limited by the sysctls, while privileged callers
    // (`CAP_SYS_RESOURCE`) may use up to the hard limits.
    let (max_maxmsg, max_msgsize) = if is_privileged {
        (HARD_MSGMAX, HARD_MSGSIZEMAX)
    } else {
        (msg_max(), msgsize_max())
    };
    if attr.mq_maxmsg > max_maxmsg || attr.mq_msgsize > max_msgsize {
        return_errno_with_message!(Errno::EINVAL, "mq_maxmsg or mq_msgsize exceeds the limit");
    }

    Ok(())
}

/// Checks whether the current thread has `CAP_SYS_RESOURCE`.
fn has_cap_sys_resource() -> bool {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        current_thread!().as_posix_thread().unwrap(),
        CapSet::SYS_RESOURCE,
    ))
    .is_ok()
}

/// Checks whether the current thread has `CAP_FOWNER`.
fn has_cap_fowner() -> bool {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        current_thread!().as_posix_thread().unwrap(),
        CapSet::FOWNER,
    ))
    .is_ok()
}

/// Returns the file system user ID of the current thread.
fn current_fsuid() -> Uid {
    current_thread!()
        .as_posix_thread()
        .unwrap()
        .credentials()
        .fsuid()
}

/// Returns the file system group ID of the current thread.
fn current_fsgid() -> Gid {
    current_thread!()
        .as_posix_thread()
        .unwrap()
        .credentials()
        .fsgid()
}

/// Parses an absolute timeout (`abs_timeout`).
///
/// `abs_timeout` is interpreted against `CLOCK_REALTIME`, matching the POSIX
/// semantics of `mq_timedsend` / `mq_timedreceive`.
pub(crate) fn parse_timeout(
    ctx: &Context,
    abs_timeout: Vaddr,
) -> Result<Option<ManagedTimeout<'static>>> {
    if abs_timeout == 0 {
        return Ok(None);
    }

    let time_spec: timespec_t = ctx.user_space().read_val(abs_timeout)?;
    let duration = Duration::try_from(time_spec)?;

    Ok(Some(ManagedTimeout::new_with_manager(
        Timeout::When(duration),
        RealTimeClock::timer_manager(),
    )))
}
