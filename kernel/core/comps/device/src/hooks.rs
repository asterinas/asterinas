// SPDX-License-Identifier: MPL-2.0

//! The bridge to the kernel crate.
//!
//! Component crates cannot depend on the kernel crate,
//! but device nodes are created by devtmpfs, which lives there.
//! The kernel crate installs an implementation of [`KernelHooks`] once devtmpfs is running.
//! Requests made before that are queued and replayed when the hooks arrive.

use alloc::{sync::Arc, vec::Vec};

use ostd::sync::Mutex;
use spin::Once;

use crate::{Error, Result, devnum::DevNodeRequest};

/// What the kernel crate provides to the device model.
///
/// Installed through [`install_hooks`] once devtmpfs is ready.
/// Until then, node creation requests are queued.
pub trait KernelHooks: Send + Sync + 'static {
    /// Creates a `/dev` node.
    ///
    /// A path with `/` in it, such as `input/event0`,
    /// means the intermediate directories are created too.
    fn create_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError>;

    /// Deletes a `/dev` node created earlier.
    /// The request is the one that created the node.
    fn delete_devnode(&self, request: &DevNodeRequest) -> core::result::Result<(), HookError>;
}

/// An error from a kernel hook.
#[derive(Clone, Copy, Debug)]
pub struct HookError;

/// Installs the kernel hooks and replays every request queued before.
///
/// Node creation requests are queued before installation;
/// removing a device cancels its queued request.
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

/// The slot the kernel crate installs into.
static HOOKS: HookSlot = HookSlot::new();

/// The hooks, once installed, and the requests waiting for them.
pub(crate) struct HookSlot {
    hooks: Once<Arc<dyn KernelHooks>>,
    pending: Mutex<Vec<DevNodeRequest>>,
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
    /// Installing, draining and replaying all happen under the queue lock,
    /// so a request made concurrently is either replayed here or delivered directly,
    /// and a removal cannot overtake the create it cancels.
    /// Calling this a second time has no effect.
    pub(crate) fn install(&self, hooks: Arc<dyn KernelHooks>) {
        let mut pending = self.pending.lock();
        self.hooks.call_once(|| hooks);
        let installed = self
            .hooks
            .get()
            .expect("the hooks were just installed here");
        for request in core::mem::take(&mut *pending) {
            // A failure here cannot be reported to the caller that queued
            // the request long ago; the node is simply absent.
            let _ = installed.create_devnode(&request);
        }
    }

    pub(crate) fn create_devnode(&self, request: DevNodeRequest) -> Result<()> {
        let mut queue = self.pending.lock();
        match self.hooks.get() {
            Some(hooks) => {
                drop(queue);
                hooks.create_devnode(&request).map_err(|_| Error::Hook)
            }
            None => {
                queue.push(request);
                Ok(())
            }
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
                queue.retain(|pending| pending != request);
                Ok(())
            }
        }
    }
}
