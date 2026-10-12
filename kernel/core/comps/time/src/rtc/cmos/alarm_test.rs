// SPDX-License-Identifier: MPL-2.0

//! Actual IRQ8, callback reentrancy and shared-port concurrency in disposable VMs.

use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::{
    cpu::{CpuId, CpuSet, num_cpus},
    early_println,
    prelude::ktest,
    smp::inter_processor_call,
};

use super::*;

static NOTIFICATIONS: AtomicUsize = AtomicUsize::new(0);
static REENTRANT: AtomicUsize = AtomicUsize::new(0);
static SMP_ERRORS: AtomicUsize = AtomicUsize::new(0);
static SMP_OPERATIONS: AtomicUsize = AtomicUsize::new(0);

const EVERY_SECOND: RtcAlarmTime = RtcAlarmTime {
    hour: None,
    minute: None,
    second: None,
};

fn wait_for(count: usize) {
    let start = read_tsc();
    while NOTIFICATIONS.load(Ordering::SeqCst) < count {
        assert!(
            read_tsc().wrapping_sub(start) < 4 * tsc_freq(),
            "IRQ8 alarm notification timed out"
        );
        core::hint::spin_loop();
    }
}

fn wait_seconds(seconds: u64) {
    let start = read_tsc();
    while read_tsc().wrapping_sub(start) < seconds * tsc_freq() {
        core::hint::spin_loop();
    }
}

fn concurrent_alarm_access() {
    let cpu = u32::from(CpuId::current_racy()) as u8;
    for _ in 0..16 {
        let time = RtcAlarmTime {
            hour: Some(cpu),
            minute: Some(30 + cpu),
            second: Some(20 + cpu),
        };
        if crate::set_rtc_alarm(time, RtcAlarmState::Disabled).is_err() {
            SMP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        match crate::read_rtc_alarm() {
            Ok(alarm)
                if alarm.state == RtcAlarmState::Disabled
                    && (0..num_cpus() as u8).any(|cpu| {
                        alarm.time
                            == RtcAlarmTime {
                                hour: Some(cpu),
                                minute: Some(30 + cpu),
                                second: Some(20 + cpu),
                            }
                    }) => {}
            _ => {
                SMP_ERRORS.fetch_add(1, Ordering::Relaxed);
            }
        }
        if crate::read_rtc().is_err() {
            SMP_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        SMP_OPERATIONS.fetch_add(3, Ordering::Relaxed);
    }
}

#[ktest]
fn hardware_alarm_irq_one_shot_reentrancy_and_smp() {
    if !ostd::boot::boot_info()
        .kernel_cmdline
        .split_ascii_whitespace()
        .any(|arg| arg == "rtc_test=destructive")
    {
        early_println!("[rtc-hw] alarm SKIPPED: requires rtc_test=destructive");
        return;
    }
    let rtc = time_test::rtc();
    let original = crate::read_rtc().unwrap();
    let (original_alarm, status_b) = {
        let mut access = rtc.access.lock();
        (
            [
                access.read_register(Register::HourAlarm as u8),
                access.read_register(Register::MinuteAlarm as u8),
                access.read_register(Register::SecondAlarm as u8),
            ],
            access.read_register(Register::StatusB as u8),
        )
    };
    NOTIFICATIONS.store(0, Ordering::SeqCst);
    REENTRANT.store(0, Ordering::SeqCst);
    crate::set_rtc_alarm_callback(Some(Arc::new(|| {
        let alarm = crate::read_rtc_alarm().unwrap();
        assert_eq!(alarm.state, RtcAlarmState::Disabled);
        assert!(alarm.pending);
        crate::read_rtc().unwrap();
        crate::set_rtc_alarm_state(RtcAlarmState::Disabled).unwrap();
        REENTRANT.fetch_add(1, Ordering::SeqCst);
        NOTIFICATIONS.fetch_add(1, Ordering::SeqCst);
    })))
    .unwrap();
    crate::set_rtc_alarm(EVERY_SECOND, RtcAlarmState::Disabled).unwrap();
    wait_seconds(2);
    assert_eq!(NOTIFICATIONS.load(Ordering::SeqCst), 0);
    assert!(!crate::take_rtc_alarm_event().unwrap());
    crate::set_rtc_alarm_state(RtcAlarmState::Enabled).unwrap();
    wait_for(1);
    assert_eq!(REENTRANT.load(Ordering::SeqCst), 1);
    assert!(crate::read_rtc_alarm().unwrap().pending);
    assert!(crate::read_rtc_alarm().unwrap().pending);
    assert!(crate::take_rtc_alarm_event().unwrap());
    assert!(!crate::take_rtc_alarm_event().unwrap());
    wait_seconds(2);
    assert_eq!(NOTIFICATIONS.load(Ordering::SeqCst), 1);
    // A replacement callback consumes its event reentrantly.
    crate::set_rtc_alarm_callback(Some(Arc::new(|| {
        assert!(crate::take_rtc_alarm_event().unwrap());
        NOTIFICATIONS.fetch_add(1, Ordering::SeqCst);
    })))
    .unwrap();
    // Exercise a specific next-second match in addition to all-wildcard matches.
    let now = crate::read_rtc().unwrap();
    let next = (now.hour as u32 * 3600 + now.minute as u32 * 60 + now.second as u32 + 2) % 86400;
    let time = RtcAlarmTime {
        hour: Some((next / 3600) as u8),
        minute: Some((next / 60 % 60) as u8),
        second: Some((next % 60) as u8),
    };
    crate::set_rtc_alarm(time, RtcAlarmState::Enabled).unwrap();
    wait_for(2);
    assert!(!crate::take_rtc_alarm_event().unwrap());
    crate::set_rtc_alarm_callback(None).unwrap();
    crate::set_rtc_alarm(EVERY_SECOND, RtcAlarmState::Enabled).unwrap();
    let start = read_tsc();
    while !crate::read_rtc_alarm().unwrap().pending {
        assert!(read_tsc().wrapping_sub(start) < 4 * tsc_freq());
    }
    assert_eq!(NOTIFICATIONS.load(Ordering::SeqCst), 2);
    // Disable preserves an event; rearm clears it and permits another IRQ.
    crate::set_rtc_alarm_state(RtcAlarmState::Disabled).unwrap();
    assert!(crate::read_rtc_alarm().unwrap().pending);
    crate::set_rtc_alarm_state(RtcAlarmState::Enabled).unwrap();
    assert!(!crate::read_rtc_alarm().unwrap().pending);
    let start = read_tsc();
    while !crate::read_rtc_alarm().unwrap().pending {
        assert!(read_tsc().wrapping_sub(start) < 4 * tsc_freq());
    }
    assert!(crate::take_rtc_alarm_event().unwrap());
    SMP_ERRORS.store(0, Ordering::Relaxed);
    SMP_OPERATIONS.store(0, Ordering::Relaxed);
    for _ in 0..8 {
        inter_processor_call(&CpuSet::new_full(), concurrent_alarm_access).wait();
    }
    assert_eq!(SMP_ERRORS.load(Ordering::Relaxed), 0);
    assert_eq!(SMP_OPERATIONS.load(Ordering::Relaxed), 384 * num_cpus());
    {
        let mut access = rtc.access.lock();
        access.read_register(Register::StatusC as u8);
        for (register, value) in [
            Register::HourAlarm,
            Register::MinuteAlarm,
            Register::SecondAlarm,
        ]
        .into_iter()
        .zip(original_alarm)
        {
            access.write_register(register as u8, value);
        }
        access.write_register(Register::StatusB as u8, status_b);
        access.alarm_pending = false;
    }
    assert!(crate::read_rtc().unwrap() >= original);
    early_println!(
        "[rtc-hw] alarm PASS: irq8=true, one_shot=true, reentrant=true, callback_removal=true, cpus={}, operations={}, restored=true",
        num_cpus(),
        SMP_OPERATIONS.load(Ordering::Relaxed)
    );
}
