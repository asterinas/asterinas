// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use crate::SystemTime;

/// Generic interface for RTC drivers
pub trait Driver {
    /// Creates a RTC driver.
    /// Returns [`Some<Self>`] on success, [`None`] otherwise (e.g. platform unsupported).
    fn try_new() -> Option<Self>
    where
        Self: Sized;

    /// Reads RTC.
    fn read_rtc(&self) -> SystemTime;
}

macro_rules! declare_rtc_drivers {
    ( $( #[cfg $cfg:tt ] $module:ident :: $name:ident),* $(,)? ) => {
        $(
            #[cfg $cfg]
            mod $module;
        )*

        pub fn init_rtc_driver() -> Arc<dyn Driver + Send + Sync> {
            // iterate all possible drivers and pick one that can be initialized
            $(
                #[cfg $cfg]
                if let Some(driver) = $module::$name::try_new() {
                    return Arc::new(driver);
                }
            )*

            log::warn!("No RTC device found, using a fallback RTC device.");

            Arc::new(RtcFallBack)
        }
    }
}

declare_rtc_drivers! {
    #[cfg(target_arch = "x86_64")] cmos::RtcCmos,
    #[cfg(target_arch = "riscv64")] goldfish::RtcGoldfish,
    #[cfg(target_arch = "loongarch64")] loongson::RtcLoongson,
}

struct RtcFallBack;

impl Driver for RtcFallBack {
    fn try_new() -> Option<Self> {
        Some(RtcFallBack)
    }

    fn read_rtc(&self) -> SystemTime {
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
}
