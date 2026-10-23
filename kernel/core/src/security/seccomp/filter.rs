// SPDX-License-Identifier: MPL-2.0

use super::{
    abi::{
        MODE_FILTER, RET_ACTION_FULL, RET_ALLOW, RET_DATA, RET_ERRNO, RET_KILL_THREAD, RET_TRACE,
        RET_TRAP, SeccompData,
    },
    cbpf::VerifiedProgram,
};
use crate::prelude::*;

// Linux's seccomp_attach_filter() charges four extra instructions per old filter:
// <https://github.com/torvalds/linux/blob/v6.18/kernel/seccomp.c>.
const MAX_INSNS_PER_PATH: usize = 32768;
const PREVIOUS_FILTER_PENALTY: usize = 4;
const MAX_ERRNO: u32 = 4095;

/// The effective seccomp policy of one thread.
#[derive(Clone, Default)]
pub(crate) struct SeccompState {
    head: Option<Arc<FilterNode>>,
}

struct FilterNode {
    program: VerifiedProgram,
    previous: Option<Arc<FilterNode>>,
    path_insns: usize,
}

/// A syscall disposition after merging every installed filter.
pub(crate) enum SeccompDecision {
    Allow,
    Errno(u32),
    Trap(u32),
    Trace(u32),
    KillThread,
    KillProcess,
}

impl SeccompState {
    pub(crate) fn mode(&self) -> u32 {
        if self.head.is_some() { MODE_FILTER } else { 0 }
    }

    pub(super) fn append(&self, program: VerifiedProgram) -> Result<Self> {
        let path_insns = program.len()
            + self
                .head
                .as_ref()
                .map_or(0, |head| head.path_insns + PREVIOUS_FILTER_PENALTY);
        if path_insns > MAX_INSNS_PER_PATH {
            return_errno_with_message!(Errno::ENOMEM, "seccomp filter chain is too long");
        }

        let head = Arc::try_new(FilterNode {
            program,
            previous: self.head.clone(),
            path_insns,
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate seccomp filter"))?;

        Ok(Self { head: Some(head) })
    }

    pub(crate) fn evaluate(&self, data: &SeccompData) -> SeccompDecision {
        let mut result = RET_ALLOW;
        let mut node = self.head.as_deref();
        while let Some(filter) = node {
            let current = filter.program.run(data);
            // Linux compares signed actions so KILL_PROCESS precedes KILL_THREAD.
            // Strict inequality retains the newest filter's data on a tie.
            if ((current & RET_ACTION_FULL) as i32) < ((result & RET_ACTION_FULL) as i32) {
                result = current;
            }
            node = filter.previous.as_deref();
        }

        match result & RET_ACTION_FULL {
            RET_ALLOW => SeccompDecision::Allow,
            RET_ERRNO => SeccompDecision::Errno((result & RET_DATA).min(MAX_ERRNO)),
            RET_TRAP => SeccompDecision::Trap(result & RET_DATA),
            RET_TRACE => SeccompDecision::Trace(result & RET_DATA),
            RET_KILL_THREAD => SeccompDecision::KillThread,
            // Unknown or unsupported actions must never permit a syscall.
            _ => SeccompDecision::KillProcess,
        }
    }
}

impl Drop for FilterNode {
    fn drop(&mut self) {
        // A valid chain can contain thousands of one-instruction programs.
        // Release its unique tail iteratively instead of recursing on the kernel stack.
        let mut previous = self.previous.take();
        while let Some(node) = previous {
            match Arc::into_inner(node) {
                Some(mut node) => previous = node.previous.take(),
                None => break,
            }
        }
    }
}
