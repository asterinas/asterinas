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

        pub fn init_rtc_driver() -> Option<Arc<dyn Driver + Send + Sync>> {
            // iterate all possible drivers and pick one that can be initialized
            $(
                #[cfg $cfg]
                if let Some(driver) = $module::$name::try_new() {
                    return Some(Arc::new(driver));
                }
            )*

            None
        }
    }
}

declare_rtc_drivers! {
    #[cfg(target_arch = "x86_64")] cmos::RtcCmos,
    #[cfg(target_arch = "riscv64")] goldfish::RtcGoldfish,
}
