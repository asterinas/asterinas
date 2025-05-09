// SPDX-License-Identifier: MPL-2.0

use super::{Process, ProcessGroup};
use crate::{
    prelude::*,
    process::{
        get_root_pid_namespace,
        pid_namespace::{NestedId, NestedIdAttachment, NestedIdAttachmentWriteGuard},
    },
};

/// The process-related attachment.
pub struct ProcessAttachment {
    thread: AttachmentReferer,
    process: AttachmentReferer,
    process_group: AttachmentReferer,
    session: AttachmentReferer,
}

enum AttachmentReferer {
    This(Option<NestedIdAttachment>),
    Thread,
    Process,
    ProcessGroup,
}

impl AttachmentReferer {
    fn ids(&self) -> Option<&NestedId> {
        let Self::This(Some(attachment)) = self else {
            return None;
        };

        Some(attachment.ids())
    }

    fn take(&mut self) -> Option<NestedIdAttachment> {
        let Self::This(option) = self else {
            return None;
        };
        option.take()
    }
}

impl ProcessAttachment {
    pub fn get_all(
        tid: &NestedId,
        process: &Process,
        process_group_mut: &mut MutexGuard<'_, Weak<ProcessGroup>>,
    ) -> Self {
        let thread = {
            let attachment = process.pid_namespace().get_attachment(tid);
            AttachmentReferer::This(attachment)
        };

        let pid = tid;
        let process_attachment = if pid == tid {
            AttachmentReferer::Thread
        } else {
            AttachmentReferer::This(process.pid_namespace().get_attachment(pid))
        };

        let pgrp = process_group_mut.upgrade().unwrap();
        let pgid = pgrp.nested_id();

        let pgrp_attachment = if pgid == tid {
            AttachmentReferer::Thread
        } else if pgid == pid {
            AttachmentReferer::Process
        } else {
            // Note that process group may not be visible in the process's PID namespace,
            // thus we get the process group from the root PID namespace.
            AttachmentReferer::This(get_root_pid_namespace().get_attachment(pgid))
        };

        let session = pgrp.session().unwrap();
        let sid = session.nested_id();

        let session_attachment = if sid == tid {
            AttachmentReferer::Thread
        } else if sid == pid {
            AttachmentReferer::Process
        } else if sid == pgid {
            AttachmentReferer::ProcessGroup
        } else {
            AttachmentReferer::This(get_root_pid_namespace().get_attachment(sid))
        };

        Self {
            thread,
            process: process_attachment,
            process_group: pgrp_attachment,
            session: session_attachment,
        }
    }

    pub fn lock<'a>(
        &'a self,
        _process_group_mut: &mut MutexGuard<'_, Weak<ProcessGroup>>,
    ) -> ProcessAttachmentGuard<'a> {
        // Lock order: We will lock the attachment with smailler `NestedId`,
        // then the attachment with bigger `NestedId`.
        // The `NestedId`'s order is determined by the ID in the root PID namespace.

        let tid = self.thread.ids();
        let pid = self.process.ids();
        let pgid = self.process_group.ids();

        // We already have the partial order:
        //     pid <= tid;
        //     sid <= pid, sid <= pgid
        // So the order between tid, pid and sid is dertermined:
        //     sid <= pid <= tid
        // Only pgid can vary for several cases(but pgid will always be greater then sid).

        let session_guard = self.lock_session();

        // `pgid` is `None`.
        let Some(pgid) = pgid else {
            let process_guard = self.lock_process();
            let thread_guard = self.lock_thread();
            return ProcessAttachmentGuard {
                attachment: self,
                thread: thread_guard,
                process: process_guard,
                process_group: None,
                session: session_guard,
            };
        };

        // `pid` is `None`.
        let Some(pid) = pid else {
            // `tid` is `None`
            let Some(tid) = tid else {
                let pgrp_guard = self.lock_process_group();
                return ProcessAttachmentGuard {
                    attachment: self,
                    thread: None,
                    process: None,
                    process_group: pgrp_guard,
                    session: session_guard,
                };
            };

            let (thread_guard, pgrp_guard) = if tid <= pgid {
                (self.lock_thread(), self.lock_process_group())
            } else {
                let pgrp = self.lock_process_group();
                let thread = self.lock_thread();
                (thread, pgrp)
            };
            return ProcessAttachmentGuard {
                attachment: self,
                thread: thread_guard,
                process: None,
                process_group: pgrp_guard,
                session: session_guard,
            };
        };

        // `tid` is None
        let Some(tid) = tid else {
            let (process_guard, pgrp_guard) = if pid < pgid {
                (self.lock_process(), self.lock_process_group())
            } else {
                let pgrp_guard = self.lock_process_group();
                let process_guard = self.lock_process();
                (process_guard, pgrp_guard)
            };

            return ProcessAttachmentGuard {
                attachment: self,
                thread: None,
                process: process_guard,
                process_group: pgrp_guard,
                session: session_guard,
            };
        };

        // All tid, pid, pgid are not None.

        let (thread_guard, process_guard, pgrp_guard) = if pgid <= pid {
            // pgid <= pid <= tid
            let pgrp_guard = self.lock_process_group();
            let process_guard = self.lock_process();
            let thread_guard = self.lock_thread();
            (thread_guard, process_guard, pgrp_guard)
        } else if pid <= pgid && pgid <= tid {
            // pid <= pgid <= tid
            let process_guard = self.lock_process();
            let pgrp_guard = self.lock_process_group();
            let thread_guard = self.lock_thread();
            (thread_guard, process_guard, pgrp_guard)
        } else if tid <= pgid {
            // pid <= tid <= pgid
            let process_guard = self.lock_process();
            let thread_guard = self.lock_thread();
            let pgrp_guard = self.lock_process_group();
            (thread_guard, process_guard, pgrp_guard)
        } else {
            unreachable!("[Internel error] tid, pid, pgid have invalid orders");
        };

        ProcessAttachmentGuard {
            attachment: self,
            thread: thread_guard,
            process: process_guard,
            process_group: pgrp_guard,
            session: session_guard,
        }
    }

    fn lock_thread(&self) -> Option<NestedIdAttachmentWriteGuard<'_>> {
        let AttachmentReferer::This(Some(attachment)) = &self.thread else {
            return None;
        };

        Some(attachment.write())
    }

    fn lock_process(&self) -> Option<NestedIdAttachmentWriteGuard<'_>> {
        let AttachmentReferer::This(Some(attachment)) = &self.process else {
            return None;
        };

        Some(attachment.write())
    }

    fn lock_process_group(&self) -> Option<NestedIdAttachmentWriteGuard<'_>> {
        let AttachmentReferer::This(Some(attachment)) = &self.process_group else {
            return None;
        };

        Some(attachment.write())
    }

    fn lock_session(&self) -> Option<NestedIdAttachmentWriteGuard<'_>> {
        let AttachmentReferer::This(Some(attachment)) = &self.session else {
            return None;
        };

        Some(attachment.write())
    }
}

pub struct ProcessAttachmentGuard<'a> {
    attachment: &'a ProcessAttachment,
    thread: Option<NestedIdAttachmentWriteGuard<'a>>,
    process: Option<NestedIdAttachmentWriteGuard<'a>>,
    process_group: Option<NestedIdAttachmentWriteGuard<'a>>,
    session: Option<NestedIdAttachmentWriteGuard<'a>>,
}

impl<'a> ProcessAttachmentGuard<'a> {
    pub fn detach_thread(&mut self) {
        self.thread.as_mut().unwrap().detach_thread();
    }

    pub fn detach_process(&mut self) {
        self.process_attachment().detach_process();
    }

    pub fn detach_process_group(&mut self) {
        self.process_group_attachment().detach_process_group();
    }

    pub fn detach_session(&mut self) {
        self.session_attachment().detach_session();
    }

    pub fn process_attachment(&mut self) -> &mut NestedIdAttachmentWriteGuard<'a> {
        match &self.attachment.process {
            AttachmentReferer::This(_) => self.process.as_mut().unwrap(),
            AttachmentReferer::Thread => self.thread.as_mut().unwrap(),
            AttachmentReferer::Process | AttachmentReferer::ProcessGroup => unreachable!(),
        }
    }

    fn process_group_attachment(&mut self) -> &mut NestedIdAttachmentWriteGuard<'a> {
        match &self.attachment.process_group {
            AttachmentReferer::This(_) => self.process_group.as_mut().unwrap(),
            AttachmentReferer::Thread => self.thread.as_mut().unwrap(),
            AttachmentReferer::Process => self.process.as_mut().unwrap(),
            AttachmentReferer::ProcessGroup => unreachable!(),
        }
    }

    fn session_attachment(&mut self) -> &mut NestedIdAttachmentWriteGuard<'a> {
        match &self.attachment.session {
            AttachmentReferer::This(_) => self.session.as_mut().unwrap(),
            AttachmentReferer::Thread => self.thread.as_mut().unwrap(),
            AttachmentReferer::Process => self.process.as_mut().unwrap(),
            AttachmentReferer::ProcessGroup => self.process_group.as_mut().unwrap(),
        }
    }
}

impl Drop for ProcessAttachment {
    fn drop(&mut self) {
        let thread_ids = self
            .thread
            .take()
            .map(|attachment| attachment.ids().clone());
        let process_ids = self
            .process
            .take()
            .map(|attachment| attachment.ids().clone());
        let pgrp_ids = self
            .process_group
            .take()
            .map(|attachment| attachment.ids().clone());
        let session_ids = self
            .session
            .take()
            .map(|attachment| attachment.ids().clone());

        let root_namespace = get_root_pid_namespace();
        if let Some(thread_ids) = thread_ids {
            root_namespace.dealloc_attachment(&thread_ids);
        }

        if let Some(process_ids) = process_ids {
            root_namespace.dealloc_attachment(&process_ids);
        }

        if let Some(pgrp_ids) = pgrp_ids {
            root_namespace.dealloc_attachment(&pgrp_ids);
        }

        if let Some(session_ids) = session_ids {
            root_namespace.dealloc_attachment(&session_ids);
        }
    }
}
