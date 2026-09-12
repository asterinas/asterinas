// SPDX-License-Identifier: MPL-2.0

//! The bridge to the kernel crate.
//!
//! Component crates cannot depend on the kernel crate, but device nodes are
//! created by devtmpfs and uevents are sent through a netlink socket, both of
//! which live there. The kernel crate installs an implementation of
//! [`KernelHooks`] once those facilities are running. Requests made before
//! that are queued and replayed when the hooks arrive.

use alloc::{sync::Arc, vec::Vec};

use ostd::sync::Mutex;
use spin::Once;

use crate::{Error, Result, devnum::DevNodeRequest, uevent::Uevent};

/// An error from a kernel hook.
#[derive(Clone, Copy, Debug)]
pub struct HookError;

/// What the kernel crate provides to the device model.
pub trait KernelHooks: Send + Sync + 'static {
    /// Creates a `/dev` node.
    ///
    /// A path with `/` in it, such as `input/event0`, means the intermediate
    /// directories are created too.
    fn create_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;

    /// Deletes a `/dev` node created earlier. The request is the one that
    /// created the node.
    fn delete_devnode(&self, request: &DevNodeRequest) -> Result<(), HookError>;

    /// Delivers a uevent to user space.
    fn broadcast_uevent(&self, event: &Uevent);
}

/// A request made before the hooks were installed.
enum Pending {
    Create(DevNodeRequest),
    Event(Uevent),
}

/// The hooks, once installed, and the requests waiting for them.
///
/// A slot exists so that the queue-and-replay behavior can be tested without
/// touching the one the kernel installs into.
pub(crate) struct HookSlot {
    hooks: Once<Arc<dyn KernelHooks>>,
    pending: Mutex<Vec<Pending>>,
}

impl HookSlot {
    pub(crate) const fn new() -> Self {
        Self {
            hooks: Once::new(),
            pending: Mutex::new(Vec::new()),
        }
    }

    /// Installs the hooks and replays every request queued before.
    ///
    /// Installing, draining and replaying all happen under the queue lock, so
    /// a request made concurrently is either replayed here or delivered
    /// directly, and a removal cannot overtake the create it cancels.
    /// Calling this a second time has no effect.
    pub(crate) fn install(&self, hooks: Arc<dyn KernelHooks>) {
        let mut pending = self.pending.lock();
        self.hooks.call_once(|| hooks);
        let installed = self
            .hooks
            .get()
            .expect("the hooks were just installed here");
        for item in core::mem::take(&mut *pending) {
            match item {
                // A failure here cannot be reported to the caller that queued
                // the request long ago; the node is simply absent.
                Pending::Create(request) => {
                    let _ = installed.create_devnode(&request);
                }
                Pending::Event(event) => installed.broadcast_uevent(&event),
            }
        }
    }

    /// Returns the installed hooks, or queues `pending` if there are none yet.
    ///
    /// The check and the push happen under the queue lock; see
    /// [`Self::install`].
    fn hooks_or_queue(&self, pending: impl FnOnce() -> Pending) -> Option<&Arc<dyn KernelHooks>> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => Some(hooks),
            None => {
                queue.push(pending());
                None
            }
        }
    }

    pub(crate) fn create_devnode(&self, request: DevNodeRequest) -> Result<()> {
        match self.hooks_or_queue(|| Pending::Create(request.clone())) {
            Some(hooks) => hooks.create_devnode(&request).map_err(|_| Error::Hook),
            None => Ok(()),
        }
    }

    pub(crate) fn delete_devnode(&self, request: &DevNodeRequest) -> Result<()> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => {
                drop(queue);
                hooks.delete_devnode(request).map_err(|_| Error::Hook)
            }
            None => {
                // The node was never created; forget the queued request.
                queue.retain(|item| !matches!(item, Pending::Create(r) if r == request));
                Ok(())
            }
        }
    }

    pub(crate) fn broadcast_uevent(&self, event: Uevent) {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => {
                drop(queue);
                hooks.broadcast_uevent(&event);
            }
            None => queue.push(Pending::Event(event)),
        }
    }
}

/// The slot the kernel crate installs into.
static HOOKS: HookSlot = HookSlot::new();

/// Installs the kernel hooks and replays every request queued before.
///
/// Calling this a second time has no effect.
pub fn install_hooks(hooks: Arc<dyn KernelHooks>) {
    HOOKS.install(hooks);
}

pub(crate) fn create_devnode(request: DevNodeRequest) -> Result<()> {
    HOOKS.create_devnode(request)
}

pub(crate) fn delete_devnode(request: &DevNodeRequest) -> Result<()> {
    HOOKS.delete_devnode(request)
}

pub(crate) fn broadcast_uevent(event: Uevent) {
    HOOKS.broadcast_uevent(event);
}
