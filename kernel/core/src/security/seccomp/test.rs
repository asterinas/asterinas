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

#[ktest]
fn strict_and_filter_modes_cannot_be_interchanged() {
    let disabled = SeccompState::default();
    let strict = disabled.to_strict().unwrap();
    assert_eq!(strict.mode(), MODE_STRICT);
    assert_eq!(strict.to_strict().unwrap().mode(), MODE_STRICT);
    assert_eq!(
        strict.append(program(RET_ALLOW)).err().unwrap().error(),
        Errno::EINVAL
    );
    let filtered = disabled.append(program(RET_ALLOW)).unwrap();
    assert_eq!(filtered.to_strict().err().unwrap().error(), Errno::EINVAL);
    assert_eq!(disabled.mode(), 0);
    assert_eq!(filtered.mode(), MODE_FILTER);
}

#[ktest]
fn strict_threads_are_not_filter_sync_ancestors() {
    let disabled = SeccompState::default();
    let strict = disabled.to_strict().unwrap();
    let filtered = disabled.append(program(RET_ALLOW)).unwrap();
    assert!(!strict.is_ancestor_of(&disabled));
    assert!(!strict.is_ancestor_of(&filtered));
    assert!(!strict.is_ancestor_of(&strict));
    assert!(disabled.is_ancestor_of(&filtered));
    assert!(filtered.is_ancestor_of(&filtered.clone()));
}

#[ktest]
fn inherited_strict_policy_retains_its_mode_and_restrictions() {
    let original = SeccompState::default().to_strict().unwrap();
    let inherited = original.clone();
    drop(original);
    assert_eq!(inherited.mode(), MODE_STRICT);
    // An invalid syscall must remain forbidden after the source state is dropped.
    let mut context = ostd::arch::cpu::context::UserContext::default();
    use crate::cpu::LinuxAbi;
    context.set_syscall_ret(usize::MAX);
    let data = SeccompData::from_context(&context, -1);
    assert!(matches!(
        inherited.evaluate(&data),
        SeccompDecision::StrictViolation
    ));
}

#[cfg(target_arch = "x86_64")]
#[ktest]
fn strict_allows_x86_probe_trampolines() {
    let strict = SeccompState::default().to_strict().unwrap();
    let context = ostd::arch::cpu::context::UserContext::default();
    // Native Linux x86 probe ABI numbers, introduced after Linux 6.8.
    for number in [335, 336] {
        assert!(matches!(
            strict.evaluate(&SeccompData::from_context(&context, number)),
            SeccompDecision::Allow
        ));
    }
}

#[ktest]
fn terminated_policies_cannot_be_reactivated() {
    let disabled = SeccompState::default();
    let active = disabled.append(program(RET_ALLOW)).unwrap();
    let mut dead = active.clone();
    dead.mark_dead();
    for policy in [
        dead.clone(),
        dead.exited(),
        active.exited(),
        disabled.exited(),
    ] {
        assert!(!policy.is_ancestor_of(&active));
        assert!(!disabled.is_ancestor_of(&policy));
        assert_eq!(policy.to_strict().err().unwrap().error(), Errno::EINVAL);
        assert_eq!(
            policy.append(program(RET_ALLOW)).err().unwrap().error(),
            Errno::EINVAL
        );
        assert!(matches!(
            policy.evaluate(&SeccompData::new_zeroed()),
            SeccompDecision::KillProcess
        ));
    }
    assert_eq!(dead.mode(), MODE_FILTER + 1);
    assert_eq!(dead.exited().mode(), dead.mode());
    assert_eq!(dead.exited().filter_count(), 1);
    assert_eq!(active.mode(), MODE_FILTER);
}
