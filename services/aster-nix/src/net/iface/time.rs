// SPDX-License-Identifier: MPL-2.0

use aster_frame::timer::read_monotonic_milli_seconds;

pub(super) fn get_network_timestamp() -> smoltcp::time::Instant {
    let millis = read_monotonic_milli_seconds();
    smoltcp::time::Instant::from_millis(millis as i64)
}
