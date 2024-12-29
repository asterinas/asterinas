// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, Ordering::Relaxed};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use int_to_c_enum::TryFromInt;
use ostd::sync::SpinLock;

pub use super::real_time::RealTimePolicy;
use crate::sched::priority::{Nice, Priority, RangedU8};

/// The User-chosen scheduling policy.
///
/// The scheduling policies are specified by the user, usually through its priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchedPolicy {
    Stop,
    RealTime {
        rt_prio: super::real_time::RtPrio,
        rt_policy: RealTimePolicy,
    },
    Fair(Nice),
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromInt)]
#[repr(u8)]
pub(super) enum SchedPolicyKind {
    Stop = 0,
    RealTime = 1,
    Fair = 2,
    Idle = 3,
}

impl From<Priority> for SchedPolicy {
    fn from(priority: Priority) -> Self {
        match priority.range().get() {
            0 => SchedPolicy::Stop,
            rt @ 1..=99 => SchedPolicy::RealTime {
                rt_prio: RangedU8::new(rt),
                rt_policy: Default::default(),
            },
            100..=139 => SchedPolicy::Fair(priority.into()),
            _ => SchedPolicy::Idle,
        }
    }
}

impl SchedPolicy {
    pub(super) fn kind(&self) -> SchedPolicyKind {
        match self {
            SchedPolicy::Stop => SchedPolicyKind::Stop,
            SchedPolicy::RealTime { .. } => SchedPolicyKind::RealTime,
            SchedPolicy::Fair(_) => SchedPolicyKind::Fair,
            SchedPolicy::Idle => SchedPolicyKind::Idle,
        }
    }
}

define_atomic_version_of_integer_like_type!(SchedPolicyKind, try_from = true, {
    #[derive(Debug)]
    pub struct AtomicSchedPolicyKind(AtomicU8);
});

impl From<SchedPolicyKind> for u8 {
    fn from(value: SchedPolicyKind) -> Self {
        value as _
    }
}

#[derive(Debug)]
pub(super) struct SchedPolicyState {
    kind: AtomicSchedPolicyKind,
    policy: SpinLock<SchedPolicy>,
}

impl SchedPolicyState {
    pub fn new(policy: SchedPolicy) -> Self {
        Self {
            kind: AtomicSchedPolicyKind::new(policy.kind()),
            policy: SpinLock::new(policy),
        }
    }

    pub fn kind(&self) -> SchedPolicyKind {
        self.kind.load(Relaxed)
    }

    pub fn get(&self) -> SchedPolicy {
        *self.policy.disable_irq().lock()
    }

    pub fn set(&self, mut policy: SchedPolicy, update: impl FnOnce(SchedPolicy)) {
        let mut this = self.policy.disable_irq().lock();

        // Keep the old base slice factor if the new policy doesn't specify one.
        if let (
            SchedPolicy::RealTime {
                rt_policy:
                    RealTimePolicy::RoundRobin {
                        base_slice_factor: slot,
                    },
                ..
            },
            SchedPolicy::RealTime {
                rt_policy: RealTimePolicy::RoundRobin { base_slice_factor },
                ..
            },
        ) = (*this, &mut policy)
        {
            *base_slice_factor = slot.or(*base_slice_factor);
        }

        update(policy);
        self.kind.store(policy.kind(), Relaxed);
        *this = policy;
    }
}
