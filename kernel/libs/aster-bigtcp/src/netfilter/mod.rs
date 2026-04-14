// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, sync::Arc, vec::Vec};

use ostd::sync::SpinLock;
use smoltcp::socket::udp::UdpMetadata;
use spin::once::Once;

pub mod ebpf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Accept,
    Drop,
}

pub struct Registry {
    udp_send: SpinLock<Vec<Box<dyn HookFunction>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            udp_send: SpinLock::new(Vec::new()),
        }
    }

    pub fn register_udp_send_hook(&mut self, hook: Box<dyn HookFunction>) {
        self.udp_send.lock().push(hook);
    }

    pub fn udp_send_hooks_exist(&self) -> bool {
        !self.udp_send.lock().is_empty()
    }

    pub fn run_udp_send_hooks(&self, context: &mut HookContext) -> Option<Verdict> {
        for hook in self.udp_send.lock().iter() {
            if let Some(verdict) = hook.run(context) {
                return Some(verdict);
            }
        }
        None
    }
}

static NETFILTER_REGISTRY: Once<Arc<Registry>> = Once::new();

pub fn init_registry() {
    NETFILTER_REGISTRY.call_once(|| Arc::new(Registry::new()));
}

pub fn registry() -> Option<&'static Arc<Registry>> {
    NETFILTER_REGISTRY.get()
}

#[repr(C)]
pub struct HookContext {
    metadata: UdpMetadata,
    packet: Vec<u8>,
}

impl HookContext {
    pub fn new(metadata: UdpMetadata, packet: Vec<u8>) -> Self {
        Self { metadata, packet }
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
