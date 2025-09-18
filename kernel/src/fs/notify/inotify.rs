// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::VecDeque,
    string::String,
    sync::{Arc, Weak},
};
use core::{
    any::Any,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

use bitflags::bitflags;
use hashbrown::HashMap;
use ostd::{mm::VmWriter, sync::Mutex};

use crate::{
    current_userspace,
    events::IoEvents,
    fs::{
        file_handle::FileLike,
        notify::{FsnotifyEvents, FsnotifySubscriber},
        path::Path,
        utils::{AccessMode, Inode, InodeMode, IoctlCmd, Metadata, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    return_errno_with_message,
};

#[derive(Clone)]
struct SubscriberEntry {
    inode: Weak<dyn Inode>,
    subscriber: Weak<dyn FsnotifySubscriber>,
}

/// A file-like object that provides inotify functionality.
///
/// InotifyFile accepts events from multiple inotify subscribers (watches) on different inodes.
/// Users should read events from this file to receive notifications about file system changes.
pub struct InotifyFile {
    // Lock to serialize subscriber updates and removals.
    subscriber_lock: Mutex<()>,
    // A subscriber descriptor allocator.
    sd_allocator: AtomicU32,
    // A map from subscriber descriptor to subscriber entry.
    sd_map: RwLock<HashMap<u32, SubscriberEntry>>,
    // Whether the file is opened in non-blocking mode.
    is_nonblocking: AtomicBool,
    // Bounded queue of inotify events.
    notifications: RwLock<VecDeque<InotifyEvent>>,
    // Maximum number of queued events.
    queue_limit: usize,
    // A weak reference to this inotify file.
    this: Weak<InotifyFile>,
    // A pollable object for this inotify file.
    pollee: Pollee,
}

impl Drop for InotifyFile {
    /// Clean up all subscribers when the inotify file is dropped.
    /// This will remove all subscribers from their inodes.
    fn drop(&mut self) {
        let sd_map = self.sd_map.write();
        for (_, entry) in sd_map.iter() {
            // The weak refs may have already been dropped by concurrent activity.
            // If so, skip them rather than unwrapping.
            if let (Some(inode), Some(subscriber)) =
                (entry.inode.upgrade(), entry.subscriber.upgrade())
            {
                inode.fsnotify_publisher().remove_subscriber(&subscriber);
            }
        }
    }
}

/// Default max queued events.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.14/source/fs/notify/inotify/inotify_user.c#L83>
const DEFAULT_MAX_QUEUED_EVENTS: usize = i32::MAX as usize;

impl InotifyFile {
    /// Create a new inotify file.
    ///
    /// The inotify file is used to watch the changes of the files.
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            sd_allocator: AtomicU32::new(1),
            sd_map: RwLock::new(HashMap::new()),
            subscriber_lock: Mutex::new(()),
            is_nonblocking: AtomicBool::new(is_nonblocking),
            notifications: RwLock::new(VecDeque::new()),
            queue_limit: DEFAULT_MAX_QUEUED_EVENTS,
            this: weak_self.clone(),
            pollee: Pollee::new(),
        })
    }

    /// Allocate a new subscriber descriptor.
    fn alloc_subscriber_id(&self) -> Result<u32> {
        if self.sd_allocator.load(Ordering::Relaxed) == u32::MAX {
            return_errno_with_message!(Errno::ENOSPC, "Inotify watches was reached limit");
        }
        Ok(self.sd_allocator.fetch_add(1, Ordering::Relaxed))
    }

    /// Find the subscriber entry by subscriber descriptor.
    fn find_subscriber_entry(&self, sd: u32) -> Option<SubscriberEntry> {
        let sd_map = self.sd_map.read();
        sd_map.get(&sd).cloned()
    }

    /// Remove the subscriber entry by subscriber descriptor.
    fn remove_subscriber_entry(&self, sd: u32) {
        let mut sd_map = self.sd_map.write();
        sd_map.remove(&sd);
    }

    /// Add the subscriber entry by subscriber descriptor.
    fn add_subscriber_entry(
        &self,
        sd: u32,
        inode: Weak<dyn Inode>,
        subscriber: Weak<dyn FsnotifySubscriber>,
    ) {
        let mut sd_map = self.sd_map.write();
        sd_map.insert(sd, SubscriberEntry { inode, subscriber });
    }

    /// Update fsnotify subscriber.
    ///
    /// If the subscriber is not found, create a new subscriber.
    /// If the subscriber is found, update the subscriber.
    pub fn update_subscriber(
        &self,
        path: &Path,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<u32> {
        // Serialize updates so concurrent callers do not create duplicate subscribers.
        let _guard = self.subscriber_lock.lock();
        // try to update subscriber with the new arg.
        let ret = self.update_existing_subscriber(path, interesting, options);
        match ret {
            Ok(sd) => Ok(sd),
            Err(e) => {
                if e.error() == Errno::ENOENT {
                    // if the subscriber is not found, create a new subscriber.
                    let sd = self.create_new_subscriber(path, interesting, options)?;
                    Ok(sd)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Remove fsnotify subscriber by subscriber descriptor.
    pub fn remove_subscriber(&self, sd: u32) -> Result<()> {
        let _guard = self.subscriber_lock.lock();
        // find the subscriber entry by the subscriber descriptor.
        let Some(entry) = self.find_subscriber_entry(sd) else {
            return_errno_with_message!(Errno::EINVAL, "watch not found");
        };
        // When concurrent removal happens, the weak refs may have already been dropped.
        // Try to upgrade the weak refs; if either side is gone, treat as
        // already-removed and return EINVAL after cleaning mapping.
        let (inode, subscriber) = match (entry.inode.upgrade(), entry.subscriber.upgrade()) {
            (Some(i), Some(s)) => (i, s),
            _ => {
                // Weak pointers could not be upgraded; remove stale mapping
                // and return EINVAL to the caller (watch considered gone).
                self.remove_subscriber_entry(sd);
                return_errno_with_message!(Errno::EINVAL, "watch not found");
            }
        };
        // Send the IN_IGNORED event before unlinking the subscriber so the
        // inotify file definitely queues the notification.
        let deliver_result = if let Some(inotify_subscriber) =
            (subscriber.as_ref() as &dyn Any).downcast_ref::<InotifySubscriber>()
        {
            inotify_subscriber.deliver_event(FsnotifyEvents::IN_IGNORED, None)
        } else {
            Ok(())
        };

        // Remove the subscriber from the inode's publisher and internal map.
        inode.fsnotify_publisher().remove_subscriber(&subscriber);
        self.remove_subscriber_entry(sd);

        deliver_result
    }

    /// Update existing fsnotify subscriber.
    fn update_existing_subscriber(
        &self,
        path: &Path,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<u32> {
        let publisher = path.inode().fsnotify_publisher();
        if let Some(subscriber) = publisher.find_inotify_subscriber(&self.this()) {
            if options.contains(InotifyControls::MASK_CREATE) {
                return_errno_with_message!(Errno::EEXIST, "watch already exists");
            }
            let inotify_subscriber = (subscriber.as_ref() as &dyn Any)
                .downcast_ref::<InotifySubscriber>()
                .ok_or(Error::with_message(Errno::EINVAL, "invalid subscriber"))?;
            return inotify_subscriber.update(interesting, options);
        }
        return_errno_with_message!(Errno::ENOENT, "watch not found");
    }

    /// Create a new fsnotify subscriber and activate it.
    fn create_new_subscriber(
        &self,
        path: &Path,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<u32> {
        let inotify_subscriber = InotifySubscriber::new(self.this(), interesting, options)?;
        // Add the subscriber to the inode's publisher.
        let subscriber = inotify_subscriber.clone() as Arc<dyn FsnotifySubscriber>;
        path.inode()
            .fsnotify_publisher()
            .add_subscriber(subscriber.clone());
        // Store the mapping between subscriber descriptor and subscriber entry.
        let sd = inotify_subscriber.sd();
        self.add_subscriber_entry(
            sd,
            Arc::downgrade(path.inode()),
            Arc::downgrade(&subscriber),
        );
        Ok(sd)
    }

    /// Send inotify event to the inotify file.
    /// The event will be queued and can be read by users.
    /// If the event can be merged with the last event, it will be merged.
    /// The event is only sent if the subscriber is interested in the event.
    fn receive_event(
        &self,
        subscriber: &InotifySubscriber,
        event: FsnotifyEvents,
        name: Option<String>,
    ) -> Result<()> {
        let sd = subscriber.sd();
        let interesting = subscriber.interesting();
        if !event.contains(FsnotifyEvents::IN_IGNORED) && !event_is_interested(event, interesting) {
            return Ok(());
        }

        let new_event = InotifyEvent::new(sd, event, 0, name);

        let mut notifications = self.notifications.write();
        if let Some(last_event) = notifications.back() {
            if can_merge_events(last_event, &new_event) {
                notifications.pop_back();
                notifications.push_back(new_event);
                drop(notifications);
                self.pollee.notify(IoEvents::IN);
                return Ok(());
            }
        }

        if notifications.len() >= self.queue_limit {
            return_errno_with_message!(Errno::ENOSPC, "inotify event queue is full");
        }

        notifications.push_back(new_event);
        drop(notifications);
        // New or merged event makes the file readable
        self.pollee.notify(IoEvents::IN);
        Ok(())
    }

    /// Pop an event from the notification queue.
    fn pop_event(&self) -> Option<InotifyEvent> {
        let mut notifications = self.notifications.write();
        notifications.pop_front()
    }

    /// Get the total size of all events in the notification queue.
    fn get_all_event_size(&self) -> usize {
        let guard = self.notifications.read();

        guard.iter().map(|event| event.get_size()).sum()
    }

    /// Try to read events from the notification queue.
    fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut size = 0;
        let mut consumed_events = 0;

        while let Some(event) = self.pop_event() {
            match event.copy_to_user(writer) {
                Ok(event_size) => {
                    size += event_size;
                    consumed_events += 1;
                }
                Err(e) => {
                    self.notifications.write().push_front(event);
                    if consumed_events == 0 {
                        return Err(e);
                    }
                    break;
                }
            }
        }

        if consumed_events == 0 {
            return_errno_with_message!(Errno::EAGAIN, "no inotify events available");
        }

        self.pollee.invalidate();
        Ok(size)
    }

    fn this(&self) -> Arc<InotifyFile> {
        self.this.upgrade().unwrap()
    }
}

impl Pollable for InotifyFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || {
            if self.get_all_event_size() > 0 {
                IoEvents::IN
            } else {
                IoEvents::empty()
            }
        })
    }
}

impl FileLike for InotifyFile {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if self.is_nonblocking.load(Ordering::SeqCst) {
            self.try_read(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.try_read(writer))
        }
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        match cmd {
            IoctlCmd::FIONREAD => {
                let size = self.get_all_event_size();
                current_userspace!().write_val(arg, &size)?;
                Ok(0)
            }
            _ => return_errno_with_message!(Errno::EINVAL, "ioctl is not supported"),
        }
    }

    fn metadata(&self) -> Metadata {
        // This is a dummy implementation.
        // TODO: Add "anonymous inode fs" and link `InotifyFile` to it.
        Metadata::new_file(
            0,
            InodeMode::from_bits_truncate(0o600),
            aster_block::BLOCK_SIZE,
        )
    }

    fn status_flags(&self) -> StatusFlags {
        if self.is_nonblocking.load(Ordering::SeqCst) {
            StatusFlags::O_NONBLOCK
        } else {
            StatusFlags::empty()
        }
    }

    fn set_status_flags(&self, new_flags: StatusFlags) -> Result<()> {
        self.is_nonblocking.store(
            new_flags.contains(StatusFlags::O_NONBLOCK),
            Ordering::SeqCst,
        );
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDONLY
    }
}

/// Check if the event type is mergeable.
fn is_mergeable_event_type(event: FsnotifyEvents) -> bool {
    event & (FsnotifyEvents::MODIFY | FsnotifyEvents::ATTRIB | FsnotifyEvents::ACCESS)
        != FsnotifyEvents::empty()
}

/// Check if two inotify events can be merged.
fn can_merge_events(existing: &InotifyEvent, new_event: &InotifyEvent) -> bool {
    existing.sd == new_event.sd
        && existing.name == new_event.name
        && existing.event == new_event.event
        && is_mergeable_event_type(new_event.event)
}

/// Inotify subscriber is used to represent a watch on a file or directory.
/// In inotify implementation, watch is equivalent to subscriber.
/// interesting is the event that the subscriber is interested in.
/// options is the control options for the subscriber.
/// Both interesting and options are stored in a single AtomicU64 for atomic updates.
pub struct InotifySubscriber {
    // interesting events and control options.
    interesting_and_controls: AtomicU64,
    // subscriber descriptor.
    sd: u32,
    // reference to the owning inotify file.
    inotify_file: Arc<InotifyFile>,
}

impl InotifySubscriber {
    /// Create a new InotifySubscriber with initial interesting events and options.
    /// The interesting_and_controls is packed into a u64: high 32 bits for options, low 32 bits for interesting.
    pub fn new(
        inotify_file: Arc<InotifyFile>,
        interesting: InotifyEvents,
        options: InotifyControls,
    ) -> Result<Arc<Self>> {
        let sd = inotify_file.alloc_subscriber_id()?;
        let this = Arc::new(Self {
            interesting_and_controls: AtomicU64::new(0),
            sd,
            inotify_file,
        });
        // Initialize the interesting_and_controls atomically
        this.update_interesting_and_controls(interesting.bits(), options.bits());
        Ok(this)
    }

    pub fn sd(&self) -> u32 {
        self.sd
    }

    fn interesting(&self) -> InotifyEvents {
        let flags = self.interesting_and_controls.load(Ordering::SeqCst);
        InotifyEvents::from_bits_truncate((flags & 0xFFFFFFFF) as u32)
    }

    fn options(&self) -> InotifyControls {
        let flags = self.interesting_and_controls.load(Ordering::SeqCst);
        InotifyControls::from_bits_truncate((flags >> 32) as u32)
    }

    pub fn inotify_file(&self) -> Arc<InotifyFile> {
        self.inotify_file.clone()
    }

    /// Update the interesting events and options atomically using CAS loop.
    fn update(&self, interesting: InotifyEvents, options: InotifyControls) -> Result<u32> {
        if options.contains(InotifyControls::MASK_CREATE) {
            return_errno_with_message!(Errno::EEXIST, "watch already exists");
        }

        let mut merged_interesting = interesting;
        let mut merged_options = options;

        if options.contains(InotifyControls::MASK_ADD) {
            merged_interesting |= self.interesting();
            merged_options |= self.options();
        }
        merged_options.remove(InotifyControls::MASK_ADD);

        self.update_interesting_and_controls(merged_interesting.bits(), merged_options.bits());
        Ok(self.sd())
    }

    /// Atomically update the interesting events and options using a CAS loop to ensure consistency.
    fn update_interesting_and_controls(&self, new_interesting: u32, new_options: u32) {
        let new_flags = ((new_options as u64) << 32) | (new_interesting as u64);
        loop {
            let old_flags = self.interesting_and_controls.load(Ordering::SeqCst);
            if self
                .interesting_and_controls
                .compare_exchange(old_flags, new_flags, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
    }
}

impl FsnotifySubscriber for InotifySubscriber {
    /// Send fsnotify event to the inotify file.
    fn deliver_event(&self, event: FsnotifyEvents, name: Option<String>) -> Result<()> {
        let inotify_file = self.inotify_file();
        inotify_file.receive_event(self, event, name)?;
        Ok(())
    }
}

/// An inotify event structure.
struct InotifyEvent {
    sd: u32,
    event: FsnotifyEvents,
    cookie: u32,
    name: Option<String>,
}

impl InotifyEvent {
    fn new(sd: u32, event: FsnotifyEvents, cookie: u32, name: Option<String>) -> Self {
        Self {
            sd,
            event,
            cookie,
            name,
        }
    }
}

impl InotifyEvent {
    fn copy_to_user(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut total_size = 0;

        let name_len = self.name.as_ref().map_or(0, |name| (name.len() + 1) as u32);

        // Write the event header
        writer.write_val(&self.sd)?;
        writer.write_val(&self.event.bits())?;
        writer.write_val(&self.cookie)?;
        writer.write_val(&name_len)?;
        total_size += core::mem::size_of::<u32>() * 4;

        if let Some(name) = self.name.as_ref() {
            for byte in name.as_bytes() {
                writer.write_val(byte)?;
            }
            writer.write_val(&b'\0')?;
            total_size += name.len() + 1;
        }

        Ok(total_size)
    }

    fn get_size(&self) -> usize {
        core::mem::size_of::<u32>() * 4 + self.name.as_ref().map_or(0, |name| name.len() + 1)
    }
}

bitflags! {
    /// InotifyEvents represents the events that the subscriber is interested in.
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

/// Check if the event is of interest to the subscriber.
fn event_is_interested(event: FsnotifyEvents, interesting: InotifyEvents) -> bool {
    event.bits() & interesting.bits() != 0
}
