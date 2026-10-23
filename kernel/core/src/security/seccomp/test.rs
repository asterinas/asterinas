// SPDX-License-Identifier: MPL-2.0

use ostd::prelude::*;

use super::{abi::*, cbpf::VerifiedProgram, filter::*};
use crate::prelude::*;

fn program(action: u32) -> VerifiedProgram {
    VerifiedProgram::new(vec![SockFilter {
        code: 0x06, // BPF_RET | BPF_K
        jt: 0,
        jf: 0,
        k: action,
    }])
    .unwrap()
}

#[ktest]
fn newest_equal_priority_data_wins() {
    let data = SeccompData::new_zeroed();
    let original = SeccompState::default()
        .append(program(RET_ERRNO | 13))
        .unwrap();
    let appended = original.append(program(RET_ERRNO | 16)).unwrap();
    assert!(matches!(
        original.evaluate(&data),
        SeccompDecision::Errno(13)
    ));
    assert!(matches!(
        appended.evaluate(&data),
        SeccompDecision::Errno(16)
    ));
    let allowed = appended.append(program(RET_ALLOW)).unwrap();
    assert!(matches!(
        allowed.evaluate(&data),
        SeccompDecision::Errno(16)
    ));
}

#[ktest]
fn signed_action_precedence_and_unknown_actions() {
    let data = SeccompData::new_zeroed();
    let policy = SeccompState::default()
        .append(program(RET_KILL_PROCESS))
        .unwrap()
        .append(program(RET_KILL_THREAD))
        .unwrap()
        .append(program(RET_ERRNO | 13))
        .unwrap();
    assert!(matches!(
        policy.evaluate(&data),
        SeccompDecision::KillProcess
    ));

    // Unknown actions are decoded after precedence selection, as in Linux.
    let unknown = SeccompState::default()
        .append(program(0x1234_0000))
        .unwrap();
    assert!(matches!(
        unknown.evaluate(&data),
        SeccompDecision::KillProcess
    ));
    let errno = unknown.append(program(RET_ERRNO | 16)).unwrap();
    assert!(matches!(errno.evaluate(&data), SeccompDecision::Errno(16)));
}

#[ktest]
fn errno_zero_and_saturation() {
    let data = SeccompData::new_zeroed();
    let policy = SeccompState::default().append(program(RET_ERRNO)).unwrap();
    assert!(matches!(policy.evaluate(&data), SeccompDecision::Errno(0)));
    let policy = policy.append(program(RET_ERRNO | 0xffff)).unwrap();
    assert!(matches!(
        policy.evaluate(&data),
        SeccompDecision::Errno(4095)
    ));
}

#[ktest]
fn maximum_depth_chain_is_bounded_and_can_be_released() {
    let mut policy = SeccompState::default();
    for _ in 0..6554 {
        policy = policy.append(program(RET_ALLOW)).unwrap();
    }
    let error = policy.append(program(RET_ALLOW)).err().unwrap();
    assert_eq!(error.error(), Errno::ENOMEM);
    let inherited = policy.clone();
    drop(policy);
    assert!(matches!(
        inherited.evaluate(&SeccompData::new_zeroed()),
        SeccompDecision::Allow
    ));
    drop(inherited);
}
