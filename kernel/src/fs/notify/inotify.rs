// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::VecDeque,
    string::String,
    sync::{Arc, Weak},
};
use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use align_ext::AlignExt;
use bitflags::bitflags;
use hashbrown::HashMap;
use ostd::{mm::VmWriter, sync::SpinLock};

use crate::{
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        file_table::FdFlags,
        notify::{FsEventSubscriber, FsEvents},
        path::Path,
        pseudofs::AnonInodeFs,
        utils::{AccessMode, CreationFlags, Inode, InodeExt, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    return_errno_with_message,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

#[derive(Clone)]
struct SubscriberEntry {
    inode: Weak<dyn Inode>,
    subscriber: Weak<InotifySubscriber>,
}

/// A file-like object that provides inotify functionality.
///
/// `InotifyFile` accepts events from multiple inotify subscribers (watches) on different inodes.
/// Users should read events from this file to receive notifications about filesystem changes.
pub struct InotifyFile {
    // The next watch descriptor to allocate.
    next_wd: AtomicU32,
    // A map from watch descriptors to subscriber entries.
    watch_map: SpinLock<HashMap<u32, SubscriberEntry>>,
    // A mutex to synchronize `read()` operations.
    read_mutex: Mutex<()>,
    // Whether the file is opened in non-blocking mode.
    is_nonblocking: AtomicBool,
    // A bounded queue of inotify events.
    event_queue: SpinLock<VecDeque<InotifyEvent>>,
    // The maximum capacity of the event queue.
    queue_capacity: usize,
    // A pollable object for this inotify file.
    pollee: Pollee,
    // A weak reference to this inotify file.
    this: Weak<InotifyFile>,
    /// The pseudo path associated with this inotify file.
    pseudo_path: Path,
}

impl Drop for InotifyFile {
    /// Cleans up all subscribers when the inotify file is dropped.
    ///
    /// This will remove all subscribers from their inodes.
    fn drop(&mut self) {
        let watch_map = self.watch_map.get_mut();

        for (_, entry) in watch_map.drain() {
            let (Some(inode), Some(subscriber)) =
                (entry.inode.upgrade(), entry.subscriber.upgrade())
            else {
                continue;
            };

            if inode
                .fs_event_publisher()
                .unwrap()
                .remove_subscriber(&(subscriber as _))
            {
                inode.fs().fs_event_subscriber_stats().remove_subscriber();
            }
        }
    }
}

/// The default maximum capacity of the event queue.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.14/source/fs/notify/inotify/inotify_user.c#L853>
const DEFAULT_MAX_QUEUED_EVENTS: usize = 16384;

impl InotifyFile {
    /// Creates a new inotify file.
    pub fn new(is_nonblocking: bool) -> Result<Arc<Self>> {
        let pseudo_path = AnonInodeFs::new_path(|_| "anon_inode:inotify".to_string());

        Ok(Arc::new_cyclic(|weak_self| Self {
            // Allocate watch descriptors from 1.
            // Reference: <https://elixir.bootlin.com/linux/v6.17/source/fs/notify/inotify/inotify_user.c#L402>
            next_wd: AtomicU32::new(1),
            watch_map: SpinLock::new(HashMap::new()),
            read_mutex: Mutex::new(()),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            event_queue: SpinLock::new(VecDeque::new()),
            queue_capacity: DEFAULT_MAX_QUEUED_EVENTS,
            pollee: Pollee::new(),
            this: weak_self.clone(),
            pseudo_path,
        }))
    }

    /// Allocates a new watch descriptor.
    fn alloc_wd(&self) -> Result<u32> {
        const MAX_VALID_WD: u32 = i32::MAX as u32;

        let new_wd = self.next_wd.fetch_add(1, Ordering::Relaxed);
        if new_wd > MAX_VALID_WD {
            // Roll back the allocation if we exceed the limit.
            self.next_wd.fetch_sub(1, Ordering::Relaxed);
            return_errno_with_message!(Errno::ENOSPC, "the inotify watch limit is reached");
        }
        Ok(new_wd)
    }

    /// Adds or updates a watch on a path.
    ///
    /// If a watch on the path is not found, creates a new watch.
    /// If a watch on the path is found, updates it.
    pub fn add_watch(
        &self,
        path: &Path,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<u32> {
        let mut watch_map = self.watch_map.lock();

        // Try to find and update the existing subscriber first.
        let inode_weak = Arc::downgrade(path.inode());
        for (wd, entry) in watch_map.iter() {
            if !Weak::ptr_eq(&entry.inode, &inode_weak) {
                continue;
            }

            // The inode has been unlinked and the subscriber is dead. We shouldn't need to update
            // since no new events can occur.
            let Some(subscriber) = entry.subscriber.upgrade() else {
                return Ok(*wd);
            };

            subscriber.update(interesting, options)?;
            path.inode()
                .fs_event_publisher()
                .unwrap()
                .update_subscriber_events();

            return Ok(*wd);
        }

        // Create a new subscriber and register it.

        let inotify_subscriber = InotifySubscriber::new(self.this(), interesting, options)?;
        let subscriber = inotify_subscriber.clone() as Arc<dyn FsEventSubscriber>;

        let inode = path.inode();
        if inode
            .fs_event_publisher_or_init()
            .add_subscriber(subscriber)
        {
            inode.fs().fs_event_subscriber_stats().add_subscriber();
        }

        let wd = inotify_subscriber.wd();
        let entry = SubscriberEntry {
            inode: inode_weak,
            subscriber: Arc::downgrade(&inotify_subscriber),
        };
        watch_map.insert(wd, entry);

        Ok(wd)
    }

    /// Removes a watch by watch descriptor.
    pub fn remove_watch(&self, wd: u32) -> Result<()> {
        let mut watch_map = self.watch_map.lock();

        let Some(entry) = watch_map.remove(&wd) else {
            return_errno_with_message!(Errno::EINVAL, "the inotify watch does not exist");
        };

        let (inode, subscriber) = match (entry.inode.upgrade(), entry.subscriber.upgrade()) {
            (Some(inode), Some(subscriber)) => (inode, subscriber),
            // The inode has been unlinked and the subscriber is dead. The watch is considered
            // removed, so we return an error.
            _ => return_errno_with_message!(Errno::EINVAL, "the inotify watch does not exist"),
        };

        if inode
            .fs_event_publisher()
            .unwrap()
            .remove_subscriber(&(subscriber as _))
        {
            inode.fs().fs_event_subscriber_stats().remove_subscriber();
        }

        Ok(())
    }

    /// Sends an inotify event to the inotify file.
    ///
    /// The event will be queued and can be read by users.
    /// If the event can be merged with the last event in the queue, it will be merged.
    /// The event is only queued if it matches one of the subscriber's interesting events.
    fn receive_event(&self, subscriber: &InotifySubscriber, event: FsEvents, name: Option<String>) {
        if !event.contains(FsEvents::IN_IGNORED) && !subscriber.is_interesting(event) {
            return;
        }

        let wd = subscriber.wd();
        let new_event = InotifyEvent::new(wd, event, 0, name);

        'notify: {
            let mut event_queue = self.event_queue.lock();
            if let Some(last_event) = event_queue.back()
                && can_merge_events(last_event, &new_event)
            {
                event_queue.pop_back();
                event_queue.push_back(new_event);
                break 'notify;
            }

            // If the queue is full, drop the event.
            // We do not return an error to the caller.
            if event_queue.len() >= self.queue_capacity {
                return;
            }

            event_queue.push_back(new_event);
        }
        // The new event or the merged event makes the file readable.
        self.pollee.notify(IoEvents::IN);
    }

    /// Pops an event from the notification queue.
    fn pop_event(&self) -> Option<InotifyEvent> {
        let mut event_queue = self.event_queue.lock();

        let event = event_queue.pop_front();
        // Invalidate when the queue is empty.
        if event_queue.is_empty() {
            self.pollee.invalidate();
        }

        event
    }

    /// Gets the total size of all events in the notification queue.
    fn get_all_event_size(&self) -> usize {
        let event_queue = self.event_queue.lock();

        event_queue.iter().map(|event| event.total_size()).sum()
    }

    /// Tries to read events from the notification queue.
    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        // This ensures that we report continuous events even when the user program attempts to
        // call `read()` concurrently.
        let _guard = self.read_mutex.lock();

        let mut size = 0;
        let mut consumed_events = 0;

        while let Some(event) = self.pop_event() {
            match event.copy_to_user(writer) {
                Ok(event_size) => {
                    size += event_size;
                    consumed_events += 1;
                }
                Err(err) => {
                    // This won't reorder events due to `_guard`.
                    self.event_queue.lock().push_front(event);
                    if consumed_events == 0 {
                        return Err(err);
                    }
                    return Ok(size);
                }
            }
        }

        if consumed_events == 0 {
            return_errno_with_message!(Errno::EAGAIN, "no inotify events are available");
        }

        Ok(size)
    }

    fn check_io_events(&self) -> IoEvents {
        if self.event_queue.lock().is_empty() {
            IoEvents::empty()
        } else {
            IoEvents::IN
        }
    }

    fn this(&self) -> Arc<InotifyFile> {
        self.this.upgrade().unwrap()
    }
}

impl Pollable for InotifyFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl FileLike for InotifyFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if self.is_nonblocking.load(Ordering::Relaxed) {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        use crate::fs::utils::ioctl_defs::GetNumBytesToRead;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetNumBytesToRead => {
                let size = self.get_all_event_size() as i32;

                cmd.write(&size)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::ENOTTY, "the ioctl command is unknown"),
        })
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking.load(Ordering::Relaxed) {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.is_nonblocking.store(
            new_flags.contains(StatusFlags::O_NONBLOCK),
            Ordering::Relaxed,
        );
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDONLY
    }

    fn path(&self) -> &Path {
        &self.pseudo_path
    }

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display> {
        struct FdInfo {
            inner: Arc<InotifyFile>,
            fd_flags: FdFlags,
        }

        impl Display for FdInfo {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut flags = self.inner.status_flags().bits() | self.inner.access_mode() as u32;
                if self.fd_flags.contains(FdFlags::CLOEXEC) {
                    flags |= CreationFlags::O_CLOEXEC.bits();
                }

                writeln!(f, "pos:\t{}", 0)?;
                writeln!(f, "flags:\t0{:o}", flags)?;
                writeln!(f, "mnt_id:\t{}", AnonInodeFs::mount_node().id())?;
                writeln!(f, "ino:\t{}", AnonInodeFs::shared_inode().ino())?;

                for (wd, entry) in self.inner.watch_map.lock().iter() {
                    let Some(inode) = entry.inode.upgrade() else {
                        continue;
                    };
                    let Some(subscriber) = entry.subscriber.upgrade() else {
                        continue;
                    };
                    let mask = subscriber.interesting_events().bits();
                    let sdev = inode.fs().sb().fsid;
                    writeln!(
                        f,
                        "inotify wd:{} ino:{:x} sdev:{:x} mask:{:x} ignored_mask:0",
                        wd,
                        inode.ino(),
                        sdev,
                        mask
                    )?;
                }

                Ok(())
            }
        }

        Box::new(FdInfo {
            inner: self,
            fd_flags,
        })
    }
}

/// Checks if the event type is mergeable.
fn is_mergeable_event_type(event: u32) -> bool {
    event & (FsEvents::MODIFY | FsEvents::ATTRIB | FsEvents::ACCESS).bits() != 0
}

/// Checks if two inotify events can be merged.
fn can_merge_events(existing: &InotifyEvent, new_event: &InotifyEvent) -> bool {
    existing.header.wd == new_event.header.wd
        && existing.name == new_event.name
        && existing.header.event == new_event.header.event
        && is_mergeable_event_type(new_event.header.event)
}

/// Represents a watch on a file or directory in the inotify system.
///
/// In the inotify implementation, a watch is equivalent to a subscriber. The subscriber
/// specifies which events it wants to monitor using `InotifyEvents`, and control options
/// using `InotifyControls`. Both the event mask and control options are stored in a single
/// `AtomicU64` for atomic updates: the high 32 bits store options, and the low 32 bits
/// store the event mask.
pub struct InotifySubscriber {
    // Interesting events and control options.
    //
    // This field is packed into a `u64`: the high 32 bits store options,
    // and the low 32 bits store interesting events.
    interesting_and_controls: AtomicU64,
    // Watch descriptor.
    wd: u32,
    // Reference to the owning inotify file.
    inotify_file: Arc<InotifyFile>,
}

impl InotifySubscriber {
    /// Creates a new `InotifySubscriber` with initial interesting events and options.
    pub fn new(
        inotify_file: Arc<InotifyFile>,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<Arc<Self>> {
        let wd = inotify_file.alloc_wd()?;
        let this = Arc::new(Self {
            interesting_and_controls: AtomicU64::new(0),
            wd,
            inotify_file,
        });
        // Initialize the `interesting_and_controls` field.
        this.update_interesting_and_controls(interesting.bits(), options.bits());
        Ok(this)
    }

    pub fn wd(&self) -> u32 {
        self.wd
    }

    fn interesting(&self) -> InotifyEvents {
        let flags = self.interesting_and_controls.load(Ordering::Relaxed);
        InotifyEvents::from_bits_truncate((flags & 0xFFFFFFFF) as u32)
    }

    fn options(&self) -> InotifyControls {
        let flags = self.interesting_and_controls.load(Ordering::Relaxed);
        InotifyControls::from_bits_truncate((flags >> 32) as u32)
    }

    pub fn inotify_file(&self) -> &Arc<InotifyFile> {
        &self.inotify_file
    }

    /// Updates the interesting events and options atomically.
    fn update(&self, interesting: InotifyEvents, options: InotifyControls) -> Result<()> {
        if options.contains(InotifyControls::MASK_CREATE) {
            return_errno_with_message!(Errno::EEXIST, "the inotify watch already exists");
        }

        let mut merged_interesting = interesting;
        let mut merged_options = options;

        if options.contains(InotifyControls::MASK_ADD) {
            merged_interesting |= self.interesting();
            merged_options |= self.options();
        }
        merged_options.remove(InotifyControls::MASK_ADD);

        self.update_interesting_and_controls(merged_interesting.bits(), merged_options.bits());

        Ok(())
    }

    /// Updates the interesting events and options atomically with raw bits.
    fn update_interesting_and_controls(&self, new_interesting: u32, new_options: u32) {
        let new_flags = ((new_options as u64) << 32) | (new_interesting as u64);
        self.interesting_and_controls
            .store(new_flags, Ordering::Relaxed);
    }

    /// Checks if the event matches the subscriber's interesting events.
    fn is_interesting(&self, event: FsEvents) -> bool {
        self.interesting().bits() & event.bits() != 0
    }
}

impl FsEventSubscriber for InotifySubscriber {
    /// Sends FS events to the inotify file.
    fn deliver_event(&self, event: FsEvents, name: Option<String>) {
        let inotify_file = self.inotify_file();
        inotify_file.receive_event(self, event, name);
    }

    /// Returns the events that this subscriber is interested in.
    fn interesting_events(&self) -> FsEvents {
        let inotify_events = self.interesting();
        FsEvents::from_bits_truncate(inotify_events.bits())
    }
}

/// Represents an inotify event that can be read by users.
struct InotifyEvent {
    header: InotifyEventHeader,
    name: Option<String>,
}

/// The header of an inotify event.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17.8/source/include/uapi/linux/inotify.h#L21>
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct InotifyEventHeader {
    wd: u32,
    event: u32,
    cookie: u32,
    name_len: u32,
}

impl InotifyEvent {
    fn new(wd: u32, event: FsEvents, cookie: u32, name: Option<String>) -> Self {
        // Calculate the actual name length including the null terminator.
        let actual_name_len = name.as_ref().map_or(0, |name| name.len() + 1);
        // Calculate the padded name length aligned to `size_of::<InotifyEventHeader>`.
        let pad_name_len = Self::round_event_name_len(actual_name_len);

        let header = InotifyEventHeader {
            wd,
            event: event.bits(),
            cookie,
            name_len: pad_name_len as u32,
        };
        Self { header, name }
    }

    /// Rounds up the name length to align with `size_of::<InotifyEventHeader>()`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.17.8/source/fs/notify/inotify/inotify_user.c#L160>
    fn round_event_name_len(name_len: usize) -> usize {
        const HEADER_SIZE: usize = size_of::<InotifyEventHeader>(); // 16 bytes
        const { assert!(HEADER_SIZE.is_power_of_two()) };
        name_len.align_up(HEADER_SIZE)
    }

    fn total_size(&self) -> usize {
        const HEADER_SIZE: usize = size_of::<InotifyEventHeader>(); // 16 bytes
        HEADER_SIZE + (self.header.name_len as usize)
    }

    fn copy_to_user(&self, writer: &mut VmWriter) -> Result<usize> {
        let total_size = self.total_size();
        if total_size > writer.avail() {
            return_errno_with_message!(Errno::EINVAL, "the buffer is too small");
        }

        // Write the header.
        writer.write_val(&self.header)?;

        let Some(name) = self.name.as_ref() else {
            debug_assert_eq!(self.header.name_len, 0);
            return Ok(total_size);
        };
        // Write the actual name bytes.
        writer.write_fallible(&mut VmReader::from(name.as_bytes()).to_fallible())?;
        // Fill remaining bytes with zeros for alignment.
        // Note that this also includes the null terminator.
        writer.fill_zeros((self.header.name_len as usize) - name.len())?;

        Ok(total_size)
    }
}

bitflags! {
    /// Represents the set of events that a subscriber wants to monitor.
    ///
    /// These events are used to filter notifications sent to the subscriber.
    pub struct InotifyEvents: u32 {
        const ACCESS        = 1 << 0;  // File was accessed
        const MODIFY        = 1 << 1;  // File was modified
        const ATTRIB        = 1 << 2;  // Metadata changed
        const CLOSE_WRITE   = 1 << 3;  // Writable file was closed
        const CLOSE_NOWRITE = 1 << 4;  // Unwritable file closed
        const OPEN          = 1 << 5;  // File was opened
        const MOVED_FROM    = 1 << 6;  // File was moved from X
        const MOVED_TO      = 1 << 7;  // File was moved to Y
        const CREATE        = 1 << 8;  // Subfile was created
        const DELETE        = 1 << 9;  // Subfile was deleted
        const DELETE_SELF   = 1 << 10; // Self was deleted
        const MOVE_SELF     = 1 << 11; // Self was moved
        const UNMOUNT       = 1 << 13; // Backing fs was unmounted
        const Q_OVERFLOW    = 1 << 14; // Event queue overflowed
        const IGNORED       = 1 << 15; // File was ignored
        const CLOSE         = Self::CLOSE_WRITE.bits() | Self::CLOSE_NOWRITE.bits(); // Close events
        const MOVE          = Self::MOVED_FROM.bits() | Self::MOVED_TO.bits();       // Move events
        const ALL_EVENTS    = Self::ACCESS.bits() | Self::MODIFY.bits() | Self::ATTRIB.bits() |
                             Self::CLOSE_WRITE.bits() | Self::CLOSE_NOWRITE.bits() | Self::OPEN.bits() |
                             Self::MOVED_FROM.bits() | Self::MOVED_TO.bits() | Self::DELETE.bits() |
                             Self::CREATE.bits() | Self::DELETE_SELF.bits() | Self::MOVE_SELF.bits();
    }
}

bitflags! {
    pub struct InotifyControls: u32 {
        const ONLYDIR       = 1 << 24; // Only watch directories
        const DONT_FOLLOW   = 1 << 25; // Don't follow symlinks
        const EXCL_UNLINK   = 1 << 26; // Exclude events on unlinked objects
        const MASK_CREATE   = 1 << 28; // Only create watches
        const MASK_ADD      = 1 << 29; // Add to existing watch mask
        const ISDIR         = 1 << 30; // Event occurred on a directory
        const ONESHOT       = 1 << 31; // Send event once
    }
}
