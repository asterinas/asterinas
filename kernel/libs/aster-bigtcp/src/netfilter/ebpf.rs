// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::String, vec::Vec};

use rbpf::EbpfVmRaw;
use smoltcp::wire::ip::Address;

use super::{HookContext, HookFunction, Verdict};

/// A Netfilter hook that runs a user-provided eBPF program.
///
/// The program is executed over the UDP payload. Its return value is mapped
/// to [`Verdict::Drop`] when `0` and [`Verdict::Accept`] otherwise, matching
/// the convention of the Linux BPF_PROG_TYPE_NETFILTER `NF_DROP` / `NF_ACCEPT`
/// return codes.
pub struct EbpfHook {
    bytecode: Vec<u8>,
}

impl EbpfHook {
    /// Creates an eBPF hook from raw bytecode.
    ///
    /// Returns an error when the rbpf verifier rejects the program.
    pub fn new(bytecode: Vec<u8>) -> core::result::Result<Self, String> {
        EbpfVmRaw::new(Some(&bytecode)).map_err(|error| format!("{error:?}"))?;
        Ok(Self { bytecode })
    }

    /// Executes the eBPF program against a buffer that contains a small
    /// serialized metadata header followed by the packet payload. Returns the
    /// raw eBPF value.
    pub fn execute_with_context(&self, context: &mut HookContext) -> u64 {
        // Serialize a compact, fixed-layout metadata header that precedes the
        // packet payload in the memory region supplied to the eBPF VM.
        // Layout (little-endian fields):
        //  - version: u8 (1)
        //  - family: u8 (4 for IPv4, 6 for IPv6, 0 unknown)
        //  - dst_port: u16 (big-endian)
        //  - src_port: u16 (big-endian) -- currently 0 (unknown)
        //  - dst_addr: 16 bytes (IPv6-style; IPv4 stored in last 4 bytes)
        //  - src_addr: 16 bytes (IPv6-style; IPv4 stored in last 4 bytes)

        let meta = context.metadata();

        let mut header = Vec::with_capacity(40);
        header.push(1u8); // version

        // family + ports
        let mut family: u8 = 0;
        let mut dst_port_be: [u8; 2] = [0, 0];
        let mut src_port_be: [u8; 2] = [0, 0];

        dst_port_be.copy_from_slice(&meta.endpoint.port.to_be_bytes());

        // src_port is not available from UdpMetadata (only local address), leave 0

        // addr slots (IPv6-style, IPv4 in last 4 bytes)
        let mut dst_addr = [0u8; 16];
        let mut src_addr = [0u8; 16];

        match meta.endpoint.addr {
            Address::Ipv4(ipv4) => {
                family = 4;
                let octs = ipv4.octets();
                dst_addr[12..16].copy_from_slice(&octs);
            }
        }

        if let Some(local) = meta.local_address {
            match local {
                smoltcp::wire::ip::Address::Ipv4(ipv4) => {
                    let octs = ipv4.octets();
                    src_addr[12..16].copy_from_slice(&octs);
                    if family == 0 {
                        family = 4;
                    }
                }
            }
        }

        header.push(family);
        header.extend_from_slice(&dst_port_be);
        header.extend_from_slice(&src_port_be);
        header.extend_from_slice(&dst_addr);
        header.extend_from_slice(&src_addr);

        // Build combined buffer: header || packet
        let mut combined = Vec::with_capacity(header.len() + context.packet_data().len());
        combined.extend_from_slice(&header);
        combined.extend_from_slice(context.packet_data());

        let vm = match EbpfVmRaw::new(Some(&self.bytecode)) {
            Ok(vm) => vm,
            Err(_) => return 0,
        };

        vm.execute_program(combined.as_mut_slice()).unwrap_or(0)
    }
}

impl HookFunction for EbpfHook {
    fn run(&self, context: &mut HookContext) -> Option<Verdict> {
        let result = self.execute_with_context(context);
        let verdict = if result == 0 {
            Verdict::Drop
        } else {
            Verdict::Accept
        };

        Some(verdict)
    }
}
