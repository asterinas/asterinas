// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use ostd::sync::SpinLock;
use smoltcp::socket::udp::UdpMetadata;
use spin::once::Once;

pub mod ebpf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Accept,
    Drop,
}

/// A unique identifier for a registered hook.
///
/// Used later to unregister the hook without scanning for identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookId(u64);

struct HookEntry {
    id: HookId,
    hook: Box<dyn HookFunction>,
}

pub struct Registry {
    next_id: AtomicU64,
    udp_send: SpinLock<Vec<HookEntry>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            udp_send: SpinLock::new(Vec::new()),
        }
    }

    /// Registers a hook function for the UDP send hook point.
    ///
    /// Returns a [`HookId`] that can be used with [`Self::unregister_udp_send_hook`]
    /// to remove the hook later (e.g., when the associated BPF link is closed).
    pub fn register_udp_send_hook(&self, hook: Box<dyn HookFunction>) -> HookId {
        let id = HookId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.udp_send.lock().push(HookEntry { id, hook });
        id
    }

    /// Unregisters a previously-registered UDP send hook.
    ///
    /// Returns `true` if a matching hook was found and removed.
    pub fn unregister_udp_send_hook(&self, id: HookId) -> bool {
        let mut hooks = self.udp_send.lock();
        if let Some(pos) = hooks.iter().position(|entry| entry.id == id) {
            hooks.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn udp_send_hooks_exist(&self) -> bool {
        !self.udp_send.lock().is_empty()
    }

    pub fn run_udp_send_hooks(&self, context: &mut HookContext) -> Option<Verdict> {
        for entry in self.udp_send.lock().iter() {
            if let Some(verdict) = entry.hook.run(context) {
                return Some(verdict);
            }
        }
        None
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

static NETFILTER_REGISTRY: Once<Arc<Registry>> = Once::new();

pub fn init_registry() {
    NETFILTER_REGISTRY.call_once(|| Arc::new(Registry::new()));
}

pub fn registry() -> Option<&'static Arc<Registry>> {
    NETFILTER_REGISTRY.get()
}

pub struct HookContext {
    metadata: UdpMetadata,
    packet: Vec<u8>,
}

impl HookContext {
    pub fn new(metadata: UdpMetadata, packet: Vec<u8>) -> Self {
        Self { metadata, packet }
    }

    pub fn into_parts(self) -> (UdpMetadata, Vec<u8>) {
        (self.metadata, self.packet)
    }

    pub fn metadata(&self) -> &UdpMetadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut UdpMetadata {
        &mut self.metadata
    }

    /// Returns a slice of the packet data.
    pub fn packet_data(&self) -> &[u8] {
        &self.packet
    }

    /// Returns a mutable slice of the packet data.
    pub fn packet_data_mut(&mut self) -> &mut [u8] {
        &mut self.packet
    }
}

pub trait HookFunction: Send + Sync {
    fn run(&self, context: &mut HookContext) -> Option<Verdict>;
}
