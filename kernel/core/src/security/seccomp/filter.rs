// SPDX-License-Identifier: MPL-2.0

use super::{
    abi::{
        MODE_FILTER, MODE_STRICT, RET_ACTION_FULL, RET_ALLOW, RET_DATA, RET_ERRNO, RET_KILL_THREAD,
        RET_LOG, RET_TRACE, RET_TRAP, RET_USER_NOTIF, SeccompData,
    },
    cbpf::VerifiedProgram,
    listener::Listener,
};
use crate::prelude::*;

// Linux's seccomp_attach_filter() charges four extra instructions per old filter:
// <https://github.com/torvalds/linux/blob/v6.18/kernel/seccomp.c>.
const MAX_INSNS_PER_PATH: usize = 32768;
const PREVIOUS_FILTER_PENALTY: usize = 4;
const MAX_ERRNO: u32 = 4095;
// Linux uses this internal mode after a filter returns a fatal action.
const MODE_DEAD: u32 = MODE_FILTER + 1;

/// The effective seccomp policy of one thread.
#[derive(Clone, Default)]
pub(crate) struct SeccompState {
    policy: Policy,
}

/// A policy attached to a thread, distinct from a temporary evaluation snapshot.
pub(crate) struct ThreadSeccompState {
    state: SeccompState,
}

impl ThreadSeccompState {
    pub(crate) fn new(state: SeccompState) -> Self {
        state.for_each_listener(|listener| listener.attach_thread());
        Self { state }
    }

    pub(crate) fn snapshot(&self) -> SeccompState {
        self.state.clone()
    }

    #[cfg(target_arch = "x86_64")]
    pub(crate) fn is_strict(&self) -> bool {
        self.state.is_strict()
    }

    pub(crate) fn mark_dead(&mut self) {
        self.state.mark_dead();
    }
}

impl Drop for ThreadSeccompState {
    fn drop(&mut self) {
        self.state
            .for_each_listener(|listener| listener.detach_thread());
    }
}

#[derive(Clone, Default)]
enum Policy {
    #[default]
    Disabled,
    Strict,
    Filter(Arc<FilterNode>),
    Dead(Arc<FilterNode>),
    Exited {
        mode: u32,
        filter_count: usize,
    },
}

struct FilterNode {
    program: VerifiedProgram,
    previous: Option<Arc<FilterNode>>,
    path_insns: usize,
    logging: FilterLogging,
    listener: Option<Arc<Listener>>,
}

/// Whether the filter requests logging of its non-ALLOW decisions.
#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum FilterLogging {
    #[default]
    Default,
    Requested,
}

/// A disposition and the log request from the winning filter.
pub(crate) struct SeccompEvaluation {
    pub(crate) decision: SeccompDecision,
    pub(super) logged_action: Option<u32>,
}

impl SeccompEvaluation {
    /// Records the decision when its action or winning filter requests logging.
    pub(crate) fn log(&self, ctx: &Context, data: &SeccompData) {
        if let Some(action) = self.logged_action {
            data.log(ctx, action);
        }
    }
}

/// A syscall disposition after merging every installed filter.
pub(crate) enum SeccompDecision {
    Allow,
    Errno(u32),
    Trap(u32),
    Trace(u32),
    UserNotification(Option<Arc<Listener>>),
    KillThread,
    KillProcess,
    StrictViolation,
}

impl SeccompState {
    pub(crate) fn mode(&self) -> u32 {
        match self.policy {
            Policy::Disabled => 0,
            Policy::Strict => MODE_STRICT,
            Policy::Filter(_) => MODE_FILTER,
            Policy::Dead(_) => MODE_DEAD,
            Policy::Exited { mode, .. } => mode,
        }
    }

    /// Returns the filter count, retaining the last count after thread exit.
    pub(crate) fn filter_count(&self) -> usize {
        if let Policy::Exited { filter_count, .. } = self.policy {
            return filter_count;
        }
        let mut count = 0;
        let mut node = self.head();
        while let Some(filter) = node {
            count += 1;
            node = filter.previous.as_ref();
        }
        count
    }

    /// Returns the status retained after thread exit without keeping its filters alive.
    pub(crate) fn exited(&self) -> Self {
        Self {
            policy: Policy::Exited {
                mode: self.mode(),
                filter_count: self.filter_count(),
            },
        }
    }

    /// Prevents further syscalls after a fatal filter decision.
    pub(crate) fn mark_dead(&mut self) {
        if let Policy::Filter(head) = &self.policy {
            self.policy = Policy::Dead(head.clone());
        }
    }

    fn is_terminated(&self) -> bool {
        matches!(self.policy, Policy::Dead(_) | Policy::Exited { .. })
    }

    pub(crate) fn is_strict(&self) -> bool {
        matches!(self.policy, Policy::Strict)
    }

    pub(super) fn to_strict(&self) -> Result<Self> {
        if !matches!(self.policy, Policy::Disabled | Policy::Strict) {
            return_errno_with_message!(
                Errno::EINVAL,
                "cannot change seccomp filter mode to strict"
            );
        }
        Ok(Self {
            policy: Policy::Strict,
        })
    }

    fn head(&self) -> Option<&Arc<FilterNode>> {
        match &self.policy {
            Policy::Filter(head) | Policy::Dead(head) => Some(head),
            _ => None,
        }
    }

    /// Checks whether this policy can receive a TSYNC update from `source`.
    ///
    /// A disabled target can receive the update. A filtered target must share
    /// a chain suffix with `source` starting at the target's current head.
    /// Strict and terminated policies are ineligible.
    pub(super) fn can_sync_from(&self, source: &Self) -> bool {
        if self.is_strict() || source.is_strict() || self.is_terminated() || source.is_terminated()
        {
            return false;
        }
        let Some(head) = self.head() else {
            return true;
        };
        let mut node = source.head();
        while let Some(filter) = node {
            if Arc::ptr_eq(head, filter) {
                return true;
            }
            node = filter.previous.as_ref();
        }
        false
    }

    fn for_each_listener(&self, visit_fn: impl Fn(&Listener)) {
        let mut node = self.head();
        while let Some(filter) = node {
            if let Some(listener) = &filter.listener {
                visit_fn(listener);
            }
            node = filter.previous.as_ref();
        }
    }

    #[cfg(ktest)]
    pub(super) fn append(&self, program: VerifiedProgram, logging: FilterLogging) -> Result<Self> {
        self.append_with_listener(program, logging, None)
    }

    pub(super) fn append_with_listener(
        &self,
        program: VerifiedProgram,
        logging: FilterLogging,
        listener: Option<Arc<Listener>>,
    ) -> Result<Self> {
        if self.is_terminated() {
            return_errno_with_message!(Errno::EINVAL, "cannot change a terminated seccomp policy");
        }
        if self.is_strict() {
            return_errno_with_message!(
                Errno::EINVAL,
                "cannot change strict seccomp mode to filter"
            );
        }
        if listener.is_some() {
            let mut node = self.head();
            while let Some(filter) = node {
                if filter
                    .listener
                    .as_ref()
                    .is_some_and(|listener| listener.is_open())
                {
                    return_errno_with_message!(
                        Errno::EBUSY,
                        "a seccomp listener is already attached"
                    );
                }
                node = filter.previous.as_ref();
            }
        }
        let path_insns = program.len()
            + self
                .head()
                .map_or(0, |head| head.path_insns + PREVIOUS_FILTER_PENALTY);
        if path_insns > MAX_INSNS_PER_PATH {
            return_errno_with_message!(Errno::ENOMEM, "seccomp filter chain is too long");
        }

        let head = Arc::try_new(FilterNode {
            program,
            previous: self.head().cloned(),
            path_insns,
            logging,
            listener,
        })
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate seccomp filter"))?;

        Ok(Self {
            policy: Policy::Filter(head),
        })
    }

    pub(crate) fn evaluate(&self, data: &SeccompData) -> SeccompEvaluation {
        if self.is_terminated() {
            return SeccompEvaluation {
                decision: SeccompDecision::KillProcess,
                logged_action: None,
            };
        }
        if self.is_strict() {
            return if crate::syscall::strict_seccomp_allows(data.syscall_number()) {
                SeccompEvaluation {
                    decision: SeccompDecision::Allow,
                    logged_action: None,
                }
            } else {
                SeccompEvaluation {
                    decision: SeccompDecision::StrictViolation,
                    logged_action: Some(RET_KILL_THREAD),
                }
            };
        }
        let mut result = RET_ALLOW;
        let mut logging = FilterLogging::Default;
        let mut listener = None;
        let mut node = self.head().map(Arc::as_ref);
        while let Some(filter) = node {
            let current = filter.program.run(data);
            // Linux compares signed actions so KILL_PROCESS precedes KILL_THREAD.
            // Strict inequality retains the newest filter's data on a tie.
            if ((current & RET_ACTION_FULL) as i32) < ((result & RET_ACTION_FULL) as i32) {
                result = current;
                logging = filter.logging;
                listener = filter.listener.as_ref();
            }
            node = filter.previous.as_deref();
        }

        let action = result & RET_ACTION_FULL;
        let decision = match action {
            RET_ALLOW | RET_LOG => SeccompDecision::Allow,
            RET_ERRNO => SeccompDecision::Errno((result & RET_DATA).min(MAX_ERRNO)),
            RET_TRAP => SeccompDecision::Trap(result & RET_DATA),
            RET_TRACE => SeccompDecision::Trace(result & RET_DATA),
            RET_USER_NOTIF => SeccompDecision::UserNotification(listener.cloned()),
            RET_KILL_THREAD => SeccompDecision::KillThread,
            // Unknown or unsupported actions must never permit a syscall.
            _ => SeccompDecision::KillProcess,
        };
        // Match Linux's default actions_logged policy. This uses the kernel
        // logger, not the Linux audit transport; no task audit rules apply.
        let logged_action = match action {
            RET_ALLOW => None,
            RET_LOG | RET_KILL_THREAD => Some(action),
            RET_ERRNO | RET_TRAP | RET_TRACE | RET_USER_NOTIF => {
                (logging == FilterLogging::Requested).then_some(action)
            }
            // Linux retains an unknown action's raw value in the log even
            // though its disposition is KILL_PROCESS.
            _ => Some(action),
        };
        SeccompEvaluation {
            decision,
            logged_action,
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
