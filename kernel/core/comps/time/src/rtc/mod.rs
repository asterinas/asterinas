// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::SystemTime;

/// An error when accessing a hardware RTC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtcError {
    /// The time component has not been initialized.
    NotInitialized,
    /// No hardware RTC is available.
    NoDevice,
    /// The RTC backend does not support the operation.
    Unsupported,
    /// The date, encoding, range, or precision is invalid for the RTC.
    InvalidTime,
    /// The RTC did not provide a stable, accessible snapshot before the deadline.
    Timeout,
}

/// A legacy RTC alarm's time-of-day match fields.
///
/// `None` matches every value of that field. These fields do not select a date;
/// an enabled alarm fires on the next match and is then disabled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtcAlarmTime {
    pub hour: Option<u8>,
    pub minute: Option<u8>,
    pub second: Option<u8>,
}

/// Whether an RTC alarm is armed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RtcAlarmState {
    Disabled,
    Enabled,
}

/// The configured RTC alarm and its latched event state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtcAlarm {
    pub time: RtcAlarmTime,
    pub state: RtcAlarmState,
    /// Remains set after delivery until consumed or the alarm is armed again.
    pub pending: bool,
}

/// A notification invoked in interrupt context after the CMOS lock is released.
///
/// The callback and its captured values must be safe to use and drop in interrupt
/// context. The callback must not sleep or wait for another CPU.
pub type RtcAlarmCallback = dyn Fn() + Send + Sync + 'static;

/// Generic interface for RTC drivers.
pub(crate) trait Driver {
    /// Creates a RTC driver.
    /// Returns [`Some<Self>`] on success, [`None`] otherwise (e.g. platform unsupported).
    fn try_new() -> Option<Self>
    where
        Self: Sized;

    /// Reads RTC.
    fn read_rtc(&self) -> Result<SystemTime, RtcError>;

    /// Sets RTC time, if supported by this backend.
    fn set_rtc(&self, _time: &SystemTime) -> Result<(), RtcError> {
        Err(RtcError::Unsupported)
    }

    fn read_alarm(&self) -> Result<RtcAlarm, RtcError> {
        Err(RtcError::Unsupported)
    }

    fn set_alarm(&self, _time: RtcAlarmTime, _state: RtcAlarmState) -> Result<(), RtcError> {
        Err(RtcError::Unsupported)
    }

    fn set_alarm_state(&self, _state: RtcAlarmState) -> Result<(), RtcError> {
        Err(RtcError::Unsupported)
    }

    fn set_alarm_callback(&self, _callback: Option<Arc<RtcAlarmCallback>>) -> Result<(), RtcError> {
        Err(RtcError::Unsupported)
    }

    fn take_alarm_event(&self) -> Result<bool, RtcError> {
        Err(RtcError::Unsupported)
    }
}

macro_rules! declare_rtc_drivers {
    ( $( #[cfg $cfg:tt ] $module:ident :: $name:ident),* $(,)? ) => {
        pub(super) fn init_rtc_driver() -> Arc<dyn Driver + Send + Sync> {
            // Iterate all possible drivers and pick one that can be initialized.
            $(
                #[cfg $cfg]
                if let Some(driver) = $module::$name::try_new() {
                    return Arc::new(driver);
                }
            )*

            ostd::warn!("No RTC device found, falling back to a dummy RTC");

            Arc::new(RtcDummy)
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod cmos;
#[cfg(target_arch = "riscv64")]
mod goldfish;
#[cfg(target_arch = "loongarch64")]
mod loongson;
#[cfg(target_arch = "aarch64")]
mod pl031;

declare_rtc_drivers! {
    #[cfg(target_arch = "x86_64")] cmos::RtcCmos,
    #[cfg(target_arch = "riscv64")] goldfish::RtcGoldfish,
    #[cfg(target_arch = "loongarch64")] loongson::RtcLoongson,
    #[cfg(target_arch = "aarch64")] pl031::RtcPl031,
}

struct RtcDummy;

impl Driver for RtcDummy {
    fn try_new() -> Option<Self> {
        Some(Self)
    }

    fn read_rtc(&self) -> Result<SystemTime, RtcError> {
        Err(RtcError::NoDevice)
    }

    fn set_rtc(&self, _time: &SystemTime) -> Result<(), RtcError> {
        Err(RtcError::NoDevice)
    }

    fn read_alarm(&self) -> Result<RtcAlarm, RtcError> {
        Err(RtcError::NoDevice)
    }

    fn set_alarm(&self, _time: RtcAlarmTime, _state: RtcAlarmState) -> Result<(), RtcError> {
        Err(RtcError::NoDevice)
    }

    fn set_alarm_state(&self, _state: RtcAlarmState) -> Result<(), RtcError> {
        Err(RtcError::NoDevice)
    }

    fn set_alarm_callback(&self, _callback: Option<Arc<RtcAlarmCallback>>) -> Result<(), RtcError> {
        Err(RtcError::NoDevice)
    }

    fn take_alarm_event(&self) -> Result<bool, RtcError> {
        Err(RtcError::NoDevice)
    }
}

pub(super) fn fallback_time() -> SystemTime {
    SystemTime {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        nanos: 0,
    }
}
