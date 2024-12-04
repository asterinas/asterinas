// SPDX-License-Identifier: MPL-2.0

use core::num::NonZero;

use super::real_time::RealTimePolicy;
use crate::sched::priority::{Nice, NiceRange, Priority, RangedU8};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum SchedPolicyKind {
    Stop,
    RealTime,
    Fair,
    Idle,
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

const TYPE_MASK: u64 = 0x0000_0000_0000_ffff;
const TYPE_SHIFT: u32 = 0;

const TYPE_STOP: u64 = 0;
const TYPE_REAL_TIME: u64 = 1;
const TYPE_FAIR: u64 = 2;
const TYPE_IDLE: u64 = 3;

const SUBTYPE_MASK: u64 = 0x0000_0000_00ff_0000;
const SUBTYPE_SHIFT: u32 = 16;

const RT_PRIO_MASK: u64 = SUBTYPE_MASK;
const RT_PRIO_SHIFT: u32 = SUBTYPE_SHIFT;

const FAIR_NICE_MASK: u64 = SUBTYPE_MASK;
const FAIR_NICE_SHIFT: u32 = SUBTYPE_SHIFT;

const RT_TYPE_MASK: u64 = 0x0000_0000_ff00_0000;
const RT_TYPE_SHIFT: u32 = 24;

const RT_TYPE_FIFO: u64 = 0;
const RT_TYPE_RR: u64 = 1;

const RT_FACTOR_MASK: u64 = 0xffff_ffff_0000_0000;
const RT_FACTOR_SHIFT: u32 = 32;

fn get(raw: u64, mask: u64, shift: u32) -> u64 {
    (raw & mask) >> shift
}

fn set(value: u64, mask: u64, shift: u32) -> u64 {
    (value << shift) & mask
}

impl SchedPolicy {
    pub(super) fn from_raw(raw: u64) -> Self {
        match get(raw, TYPE_MASK, TYPE_SHIFT) {
            TYPE_STOP => SchedPolicy::Stop,
            TYPE_REAL_TIME => SchedPolicy::RealTime {
                rt_prio: RangedU8::new(get(raw, RT_PRIO_MASK, RT_PRIO_SHIFT) as u8),
                rt_policy: match get(raw, RT_TYPE_MASK, RT_TYPE_SHIFT) {
                    RT_TYPE_FIFO => RealTimePolicy::Fifo,
                    RT_TYPE_RR => RealTimePolicy::RoundRobin {
                        base_slice_factor: NonZero::new(
                            get(raw, RT_FACTOR_MASK, RT_FACTOR_SHIFT) as u32
                        ),
                    },
                    _ => unreachable!(),
                },
            },
            TYPE_FAIR => {
                SchedPolicy::Fair(Nice::new(NiceRange::new(
                    get(raw, FAIR_NICE_MASK, FAIR_NICE_SHIFT) as i8,
                )))
            }
            TYPE_IDLE => SchedPolicy::Idle,
            _ => unreachable!(),
        }
    }

    pub(super) fn into_raw(this: Self) -> u64 {
        match this {
            SchedPolicy::Stop => set(TYPE_STOP, TYPE_MASK, TYPE_SHIFT),
            SchedPolicy::RealTime { rt_prio, rt_policy } => {
                let ty = set(TYPE_REAL_TIME, TYPE_MASK, TYPE_SHIFT);
                let rt_prio = set(rt_prio.get() as u64, RT_PRIO_MASK, RT_PRIO_SHIFT);
                let rt_policy = match rt_policy {
                    RealTimePolicy::Fifo => set(RT_TYPE_FIFO, RT_TYPE_MASK, RT_TYPE_SHIFT),
                    RealTimePolicy::RoundRobin { base_slice_factor } => {
                        let rt_type = set(RT_TYPE_RR, RT_TYPE_MASK, RT_TYPE_SHIFT);
                        let rt_factor = set(
                            base_slice_factor.map_or(0, NonZero::get) as u64,
                            RT_FACTOR_MASK,
                            RT_FACTOR_SHIFT,
                        );
                        rt_type | rt_factor
                    }
                };
                ty | rt_prio | rt_policy
            }
            SchedPolicy::Fair(nice) => {
                let ty = set(TYPE_FAIR, TYPE_MASK, TYPE_SHIFT);
                let nice = set(nice.range().get() as u64, FAIR_NICE_MASK, FAIR_NICE_SHIFT);
                ty | nice
            }
            SchedPolicy::Idle => set(TYPE_IDLE, TYPE_MASK, TYPE_SHIFT),
        }
    }
}

impl SchedPolicyKind {
    pub fn from_raw(raw: u64) -> Self {
        match get(raw, TYPE_MASK, TYPE_SHIFT) {
            TYPE_STOP => SchedPolicyKind::Stop,
            TYPE_REAL_TIME => SchedPolicyKind::RealTime,
            TYPE_FAIR => SchedPolicyKind::Fair,
            TYPE_IDLE => SchedPolicyKind::Idle,
            _ => unreachable!(),
        }
    }
}
