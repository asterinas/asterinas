// SPDX-License-Identifier: MPL-2.0

//! POSIX message queue.

// TODO: Remove this once the message-passing methods are implemented.
#![allow(dead_code)]

mod queue;

pub(crate) use queue::{MqAttr, MqueueQueue};

use spin::Once;

use crate::{fs::file::InodeMode, prelude::*};

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
/// Queues that are still open keep their state alive until the last file
/// descriptor is closed, matching the Linux behavior of `mq_unlink`.
pub(crate) fn unlink_queue(name: &str) -> Result<()> {
    validate_name(name)?;

    let mut table = queue_table().lock();
    if table.remove(name).is_none() {
        return_errno!(Errno::ENOENT);
    }
    Ok(())
}

/// Creates a new queue with the given permission and attributes and registers it.
pub(crate) fn create_queue(
    name: &str,
    mode: InodeMode,
    attr: MqAttr,
) -> Result<Arc<MqueueQueue>> {
    validate_name(name)?;

    let mut table = queue_table().lock();
    if table.contains_key(name) {
        return_errno!(Errno::EEXIST);
    }

    let queue = Arc::new(MqueueQueue::new(mode, attr.mq_maxmsg, attr.mq_msgsize));
    table.insert(name.to_string(), queue.clone());
    Ok(queue)
}

