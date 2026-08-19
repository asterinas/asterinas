// SPDX-License-Identifier: MPL-2.0

//! LSM hook points.

mod alien_access;
mod capability;

pub(crate) use self::{
    alien_access::{AlienAccessContext, on_alien_access},
    capability::{CapableContext, on_capable},
};
use crate::{prelude::*, process::posix_thread::PosixThread};

pub(super) trait LsmAlienAccessHook: Sync {
    /// Handles an alien access attempt.
    fn on_alien_access(&self, _context: &AlienAccessContext) -> Result<()> {
        Ok(())
    }
}

pub(super) trait LsmCapabilityHook: Sync {
    /// Checks whether a thread holds a capability in a user namespace.
    fn on_capable(&self, _context: &CapableContext) -> Result<()> {
        Ok(())
    }
}

pub(crate) struct ThreadInitContext<'a> {
    task: &'a PosixThread,
}

impl<'a> ThreadInitContext<'a> {
    pub(crate) const fn new(task: &'a PosixThread) -> Self {
        Self { task }
    }

    pub(crate) const fn task(&self) -> &'a PosixThread {
        self.task
    }
}

pub(crate) struct ThreadCloneContext<'a> {
    parent: &'a PosixThread,
    child: &'a PosixThread,
}

impl<'a> ThreadCloneContext<'a> {
    pub(crate) const fn new(parent: &'a PosixThread, child: &'a PosixThread) -> Self {
        Self { parent, child }
    }

    pub(crate) const fn parent(&self) -> &'a PosixThread {
        self.parent
    }

    pub(crate) const fn child(&self) -> &'a PosixThread {
        self.child
    }
}

pub(crate) struct ThreadExecContext<'a> {
    task: &'a PosixThread,
    executable_path: &'a [u8],
}

impl<'a> ThreadExecContext<'a> {
    pub(crate) const fn new(task: &'a PosixThread, executable_path: &'a [u8]) -> Self {
        Self {
            task,
            executable_path,
        }
    }

    pub(crate) const fn task(&self) -> &'a PosixThread {
        self.task
    }

    pub(crate) const fn executable_path(&self) -> &'a [u8] {
        self.executable_path
    }
}

pub(crate) trait LsmTaskHook: Sync {
    fn on_task_init(&self, _context: &ThreadInitContext) -> Result<()> {
        Ok(())
    }

    fn on_task_clone(&self, _context: &ThreadCloneContext) -> Result<()> {
        Ok(())
    }

    fn on_task_exec(&self, _context: &ThreadExecContext) {}
}

pub(crate) fn on_task_init(context: ThreadInitContext) -> Result<()> {
    for module in super::modules::active_modules() {
        module.on_task_init(&context)?;
    }

    Ok(())
}

pub(crate) fn on_task_clone(context: ThreadCloneContext) -> Result<()> {
    for module in super::modules::active_modules() {
        module.on_task_clone(&context)?;
    }

    Ok(())
}

pub(crate) fn on_task_exec(context: ThreadExecContext) {
    for module in super::modules::active_modules() {
        module.on_task_exec(&context);
    }
}
