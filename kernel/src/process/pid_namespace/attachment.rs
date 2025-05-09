// SPDX-License-Identifier: MPL-2.0

use core::ops::{Deref, DerefMut};

use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard, RwMutexReadGuard};

use super::{NestedId, PidEvent};
use crate::{
    events::{Observer, Subject},
    prelude::*,
    process::{Process, ProcessGroup, Session},
    thread::Thread,
};

pub struct NestedIdAttachmentInner {
    pub(super) ids: NestedId,
    pub(super) attachment: RwLock<Attachment>,
}

impl NestedIdAttachmentInner {
    pub(super) const fn new(ids: NestedId) -> Self {
        let attachment = Attachment {
            thread: None,
            process: None,
            process_group: None,
            session: None,
            subject: Subject::new(),
        };
        Self {
            ids,
            attachment: RwLock::new(attachment),
        }
    }

    pub fn ids(&self) -> &NestedId {
        &self.ids
    }
}

pub struct Attachment {
    pub(super) thread: Option<Arc<Thread>>,
    pub(super) process: Option<Arc<Process>>,
    pub(super) process_group: Option<Arc<ProcessGroup>>,
    pub(super) session: Option<Arc<Session>>,
    subject: Subject<PidEvent>,
}

impl Attachment {
    pub fn attached_thread(&self) -> Option<&Arc<Thread>> {
        self.thread.as_ref()
    }

    pub fn attached_process(&self) -> Option<&Arc<Process>> {
        self.process.as_ref()
    }

    pub fn attached_process_group(&self) -> Option<&Arc<ProcessGroup>> {
        self.process_group.as_ref()
    }

    pub fn attached_session(&self) -> Option<&Arc<Session>> {
        self.session.as_ref()
    }

    pub fn attach_thread(&mut self, thread: Arc<Thread>) {
        debug_assert!(self.thread.is_none());
        self.thread = Some(thread);
    }

    pub fn attach_process(&mut self, process: Arc<Process>) {
        debug_assert!(self.process.is_none());
        self.process = Some(process);
    }

    pub fn attach_process_group(&mut self, process_group: Arc<ProcessGroup>) {
        debug_assert!(self.process_group.is_none());
        self.process_group = Some(process_group);
    }

    pub fn attach_session(&mut self, session: Arc<Session>) {
        debug_assert!(self.session.is_none());
        self.session = Some(session);
    }

    pub fn detach_thread(&mut self) {
        debug_assert!(self.thread.is_some());
        self.thread = None;
    }

    pub fn detach_process(&mut self) {
        debug_assert!(self.process.is_some());
        self.process = None;
        self.subject.notify_observers(&PidEvent::Exit);
    }

    pub fn detach_process_group(&mut self) {
        debug_assert!(self.process_group.is_some());
        self.process_group = None;
    }

    pub fn detach_session(&mut self) {
        debug_assert!(self.session.is_some());
        self.session = None;
    }

    pub(super) fn has_attached(&self) -> bool {
        self.thread.is_some()
            || self.process.is_some()
            || self.process_group.is_some()
            || self.session.is_some()
    }

    pub fn register_observer(&self, observer: Weak<dyn Observer<PidEvent>>) {
        self.subject.register_observer(observer, ());
    }
}
pub struct NestedIdAttachmentReadGuard<'a>(RwLockReadGuard<'a, Attachment, PreemptDisabled>);
pub struct NestedIdAttachmentWriteGuard<'a>(RwLockWriteGuard<'a, Attachment, PreemptDisabled>);

impl NestedIdAttachmentInner {
    pub fn read(&self) -> NestedIdAttachmentReadGuard<'_> {
        NestedIdAttachmentReadGuard(self.attachment.read())
    }

    pub fn write(&self) -> NestedIdAttachmentWriteGuard<'_> {
        NestedIdAttachmentWriteGuard(self.attachment.write())
    }
}

impl Deref for NestedIdAttachmentReadGuard<'_> {
    type Target = Attachment;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Deref for NestedIdAttachmentWriteGuard<'_> {
    type Target = Attachment;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for NestedIdAttachmentWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A guard for the [`NestedIdAttachment`].
///
/// Holding this guard ensures that the associated [`NestedIdAttachment`]
/// exists within the PID namespaces to which it belongs.
pub struct NestedIdAttachment {
    pub(super) _guard: RwMutexReadGuard<'static, ()>,
    pub(super) inner: Arc<NestedIdAttachmentInner>,
}

impl Deref for NestedIdAttachment {
    type Target = Arc<NestedIdAttachmentInner>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub(super) static GLOBAL_ATTACHMENT_LOCK: RwMutex<()> = RwMutex::new(());
