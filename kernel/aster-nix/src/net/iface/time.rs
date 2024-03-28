// SPDX-License-Identifier: MPL-2.0

use aster_frame::arch::jiffies_as_duration;

pub(super) fn get_network_timestamp() -> smoltcp::time::Instant {
    let millis = jiffies_as_duration().as_millis();
    smoltcp::time::Instant::from_millis(millis as i64)
}
