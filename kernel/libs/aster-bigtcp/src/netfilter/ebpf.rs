// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::String, vec::Vec};

use rbpf::EbpfVmRaw;

use super::{HookContext, HookFunction, Verdict};

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
