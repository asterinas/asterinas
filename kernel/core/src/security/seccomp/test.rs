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
        .append(program(RET_ERRNO | 13), FilterLogging::Default)
        .unwrap();
    let appended = original
        .append(program(RET_ERRNO | 16), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        original.evaluate(&data).decision(),
        SeccompDecision::Errno(13)
    ));
    assert!(matches!(
        appended.evaluate(&data).decision(),
        SeccompDecision::Errno(16)
    ));
    let allowed = appended
        .append(program(RET_ALLOW), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        allowed.evaluate(&data).decision(),
        SeccompDecision::Errno(16)
    ));
}

#[ktest]
fn signed_action_precedence_and_unknown_actions() {
    let data = SeccompData::new_zeroed();
    let policy = SeccompState::default()
        .append(program(RET_KILL_PROCESS), FilterLogging::Default)
        .unwrap()
        .append(program(RET_KILL_THREAD), FilterLogging::Default)
        .unwrap()
        .append(program(RET_ERRNO | 13), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        policy.evaluate(&data).decision(),
        SeccompDecision::KillProcess
    ));

    // Unknown actions are decoded after precedence selection, as in Linux.
    let unknown = SeccompState::default()
        .append(program(0x1234_0000), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        unknown.evaluate(&data).decision(),
        SeccompDecision::KillProcess
    ));
    let errno = unknown
        .append(program(RET_ERRNO | 16), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        errno.evaluate(&data).decision(),
        SeccompDecision::Errno(16)
    ));
}

#[ktest]
fn errno_zero_and_saturation() {
    let data = SeccompData::new_zeroed();
    let policy = SeccompState::default()
        .append(program(RET_ERRNO), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        policy.evaluate(&data).decision(),
        SeccompDecision::Errno(0)
    ));
    let policy = policy
        .append(program(RET_ERRNO | 0xffff), FilterLogging::Default)
        .unwrap();
    assert!(matches!(
        policy.evaluate(&data).decision(),
        SeccompDecision::Errno(4095)
    ));
}

#[ktest]
fn maximum_depth_chain_is_bounded_and_can_be_released() {
    let mut policy = SeccompState::default();
    for _ in 0..6554 {
        policy = policy
            .append(program(RET_ALLOW), FilterLogging::Default)
            .unwrap();
    }
    let error = policy
        .append(program(RET_ALLOW), FilterLogging::Default)
        .err()
        .unwrap();
    assert_eq!(error.error(), Errno::ENOMEM);
    let inherited = policy.clone();
    drop(policy);
    assert!(matches!(
        inherited.evaluate(&SeccompData::new_zeroed()).decision(),
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
        strict
            .append(program(RET_ALLOW), FilterLogging::Default)
            .err()
            .unwrap()
            .error(),
        Errno::EINVAL
    );
    let filtered = disabled
        .append(program(RET_ALLOW), FilterLogging::Default)
        .unwrap();
    assert_eq!(filtered.to_strict().err().unwrap().error(), Errno::EINVAL);
    assert_eq!(disabled.mode(), 0);
    assert_eq!(filtered.mode(), MODE_FILTER);
}

#[ktest]
fn strict_threads_are_ineligible_for_filter_sync() {
    let disabled = SeccompState::default();
    let strict = disabled.to_strict().unwrap();
    let filtered = disabled
        .append(program(RET_ALLOW), FilterLogging::Default)
        .unwrap();
    assert!(!strict.can_sync_from(&disabled));
    assert!(!strict.can_sync_from(&filtered));
    assert!(!strict.can_sync_from(&strict));
    assert!(disabled.can_sync_from(&filtered));
    assert!(filtered.can_sync_from(&filtered.clone()));
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
        inherited.evaluate(&data).decision(),
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
            strict
                .evaluate(&SeccompData::from_context(&context, number))
                .decision(),
            SeccompDecision::Allow
        ));
    }
}

#[ktest]
fn terminated_policies_cannot_be_reactivated() {
    let disabled = SeccompState::default();
    let active = disabled
        .append(program(RET_ALLOW), FilterLogging::Default)
        .unwrap();
    let mut dead = active.clone();
    dead.mark_dead();
    for policy in [
        dead.clone(),
        dead.exited(),
        active.exited(),
        disabled.exited(),
    ] {
        assert!(!policy.can_sync_from(&active));
        assert!(!disabled.can_sync_from(&policy));
        assert_eq!(policy.to_strict().err().unwrap().error(), Errno::EINVAL);
        assert_eq!(
            policy
                .append(program(RET_ALLOW), FilterLogging::Default)
                .err()
                .unwrap()
                .error(),
            Errno::EINVAL
        );
        assert!(matches!(
            policy.evaluate(&SeccompData::new_zeroed()).decision(),
            SeccompDecision::KillProcess
        ));
    }
    assert_eq!(dead.mode(), MODE_FILTER + 1);
    assert_eq!(dead.exited().mode(), dead.mode());
    assert_eq!(dead.exited().filter_count(), 1);
    assert_eq!(active.mode(), MODE_FILTER);
}

#[ktest]
fn log_action_allows_with_data_and_respects_precedence() {
    let data = SeccompData::new_zeroed();
    let policy = SeccompState::default()
        .append(program(RET_LOG | 123), FilterLogging::Default)
        .unwrap();
    let evaluation = policy.evaluate(&data);
    assert!(matches!(evaluation.decision(), SeccompDecision::Allow));
    assert_eq!(evaluation.logged_action, Some(RET_LOG));
    let policy = policy
        .append(program(RET_ALLOW), FilterLogging::Requested)
        .unwrap();
    assert_eq!(policy.evaluate(&data).logged_action, Some(RET_LOG));
    let policy = policy
        .append(program(RET_ERRNO | 13), FilterLogging::Default)
        .unwrap();
    let evaluation = policy.evaluate(&data);
    assert!(matches!(evaluation.decision(), SeccompDecision::Errno(13)));
    assert_eq!(evaluation.logged_action, None);
}

#[ktest]
fn only_winning_filter_requests_logging() {
    let data = SeccompData::new_zeroed();
    for action in [RET_ERRNO, RET_TRAP, RET_TRACE] {
        let policy = SeccompState::default()
            .append(program(action), FilterLogging::Requested)
            .unwrap();
        assert_eq!(policy.evaluate(&data).logged_action, Some(action));
        let policy = policy
            .append(program(action | 42), FilterLogging::Default)
            .unwrap();
        assert_eq!(policy.evaluate(&data).logged_action, None);
        let policy = policy
            .append(program(RET_ALLOW), FilterLogging::Requested)
            .unwrap();
        assert_eq!(policy.evaluate(&data).logged_action, None);
        let policy = policy
            .append(program(action), FilterLogging::Requested)
            .unwrap();
        let inherited = policy.clone();
        drop(policy);
        assert_eq!(inherited.evaluate(&data).logged_action, Some(action));
    }
    let allow = SeccompState::default()
        .append(program(RET_ALLOW), FilterLogging::Requested)
        .unwrap();
    assert_eq!(allow.evaluate(&data).logged_action, None);
}

#[ktest]
fn fatal_actions_log_without_filter_flag() {
    let data = SeccompData::new_zeroed();
    for (action, expected) in [
        (RET_KILL_THREAD, RET_KILL_THREAD),
        (RET_KILL_PROCESS, RET_KILL_PROCESS),
        (0x1234_0000, 0x1234_0000),
    ] {
        let policy = SeccompState::default()
            .append(program(action), FilterLogging::Default)
            .unwrap();
        assert_eq!(policy.evaluate(&data).logged_action, Some(expected));
    }
    let strict = SeccompState::default().to_strict().unwrap();
    let context = ostd::arch::cpu::context::UserContext::default();
    assert_eq!(
        strict
            .evaluate(&SeccompData::from_context(&context, -1))
            .logged_action,
        Some(RET_KILL_THREAD)
    );
}

#[ktest]
fn listener_hup_counts_threads_not_policy_snapshots() {
    use super::listener::{self, ListenerFile, NotificationWaitMode};
    use crate::{events::IoEvents, process::signal::Pollable};

    listener::init();
    let file = ListenerFile::new(NotificationWaitMode::Interruptible).unwrap();
    let listener = file.listener();
    let state = SeccompState::default()
        .append_with_listener(
            program(RET_ALLOW),
            FilterLogging::Default,
            Some(listener.clone()),
        )
        .unwrap();
    let first = ThreadSeccompState::new(state.clone());
    let second = ThreadSeccompState::new(first.snapshot());
    let snapshot = first.snapshot();
    assert!(listener.poll(IoEvents::HUP, None).is_empty());
    drop(first);
    assert!(listener.poll(IoEvents::HUP, None).is_empty());
    // An appended policy must keep the listener active while replacing its ancestor.
    let appended = ThreadSeccompState::new(
        second
            .snapshot()
            .append(program(RET_ALLOW), FilterLogging::Default)
            .unwrap(),
    );
    drop(second);
    assert!(listener.poll(IoEvents::HUP, None).is_empty());
    drop(appended);
    assert_eq!(listener.poll(IoEvents::HUP, None), IoEvents::HUP);
    assert_eq!(snapshot.filter_count(), 1);
    assert_eq!(state.filter_count(), 1);
}

#[ktest]
fn notification_uses_only_the_winning_filters_listener_and_log_flag() {
    use super::listener::{self, ListenerFile, NotificationWaitMode};

    listener::init();
    let file = ListenerFile::new(NotificationWaitMode::Interruptible).unwrap();
    let listener = file.listener();
    let policy = SeccompState::default()
        .append_with_listener(
            program(RET_USER_NOTIF | 0xffff),
            FilterLogging::Requested,
            Some(listener.clone()),
        )
        .unwrap()
        .append(program(RET_TRACE), FilterLogging::Default)
        .unwrap();
    let data = SeccompData::new_zeroed();
    let result = policy.evaluate(&data);
    assert!(
        matches!(result.decision(), SeccompDecision::UserNotification(Some(selected))
        if Arc::ptr_eq(selected, listener))
    );
    assert_eq!(result.logged_action, Some(RET_USER_NOTIF));

    // An equal-priority newer filter without a listener must not borrow an
    // older filter's listener, even though that listener is still open.
    let policy = policy
        .append(program(RET_USER_NOTIF), FilterLogging::Default)
        .unwrap();
    let result = policy.evaluate(&data);
    assert!(matches!(
        result.decision(),
        SeccompDecision::UserNotification(None)
    ));
    assert_eq!(result.logged_action, None);
}
