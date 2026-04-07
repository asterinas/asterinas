// SPDX-License-Identifier: MPL-2.0

//! Unimplemented kernel command-line parameters and placeholder handlers.
//!
//! This module declares kernel parameters that the framework recognizes but does
//! not implement behavior for yet. Parameters registered here are consumed by
//! the dispatcher (they are not forwarded to the init process) and a warning
//! is logged when they are present.

/// Defines kernel command-line parameters that are intentionally left unimplemented.
///
/// Matching tokens are consumed (not forwarded to `init`) and a warning is logged when
/// such parameters appear.
///
/// # Examples
///
/// ```ignore
/// define_unimplemented_param!("foo", "bar");
/// ```
#[macro_export]
macro_rules! define_unimplemented_param {
    ($($name:expr),+ $(,)?) => {
        $(
            const _: () = {
                fn __kparam_setup(occurrences: &[Option<&str>]) {
                    $crate::setup_unimplemented(occurrences, $name);
                }
                $crate::submit! {
                    $crate::KernelParam::new($name, __kparam_setup, false)
                }
            };
        )+
    };
}

#[doc(hidden)]
pub fn setup_unimplemented(occurrences: &[Option<&str>], name: &str) {
    if !occurrences.is_empty() {
        ostd::warn!("kernel parameter '{}' is not yet implemented", name);
    }
}

// Placeholders for recognized but unimplemented kernel command-line parameters.
define_unimplemented_param!(
    "tsc",
    "no_timer_check",
    "reboot",
    "pci",
    "debug",
    "panic",
    "nr_cpus",
    "selinux",
    "initrd",
    "noreplace_smp",
    "initcall_debug"
);
