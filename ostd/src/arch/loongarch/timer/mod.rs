// SPDX-License-Identifier: MPL-2.0

//! The timer support.

/// The timer frequency (Hz). Here we choose 1000Hz since 1000Hz is easier for
/// unit conversion and convenient for timer. What's more, the frequency cannot
/// be set too high or too low, 1000Hz is a modest choice.
///
/// For system performance reasons, this rate cannot be set too high, otherwise
/// most of the time is spent executing timer code.
pub const TIMER_FREQ: u64 = 1000;
