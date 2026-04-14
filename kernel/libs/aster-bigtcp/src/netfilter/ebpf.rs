// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::String, vec::Vec};

use rbpf::EbpfVmRaw;

use super::{HookContext, HookFunction, Verdict};

const UDP_SEND_PREFIX_A_BYTECODE: &[u8] = &[
    0x71, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ldxb r0, [r1 + 0]
    0x15, 0x00, 0x02, 0x00, 0x61, 0x00, 0x00, 0x00, // jeq r0, 'a', +2
    0xb7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mov64 r0, 0
    0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // exit
    0xb7, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, // mov64 r0, 1
    0x95, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // exit
];

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

    pub(crate) fn builtin_udp_send_prefix_a() -> Self {
        debug_assert!(EbpfVmRaw::new(Some(UDP_SEND_PREFIX_A_BYTECODE)).is_ok());
        Self {
            bytecode: UDP_SEND_PREFIX_A_BYTECODE.to_vec(),
        }
    }

    /// Executes the eBPF program against packet data and returns the raw eBPF value.
    pub fn execute(&self, packet_data: &mut [u8]) -> u64 {
        let vm = match EbpfVmRaw::new(Some(&self.bytecode)) {
            Ok(vm) => vm,
            Err(_) => return 0,
        };

        vm.execute_program(packet_data).unwrap_or(0)
    }
}

impl HookFunction for EbpfHook {
    fn run(&self, context: &mut HookContext) -> Option<Verdict> {
        let result = self.execute(context.packet_data_mut());
        let verdict = if result == 0 {
            Verdict::Drop
        } else {
            Verdict::Accept
        };

        Some(verdict)
    }
}
