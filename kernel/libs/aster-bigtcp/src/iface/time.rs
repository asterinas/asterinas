// SPDX-License-Identifier: MPL-2.0

use ostd::timer::Jiffies;

pub(super) fn get_network_timestamp() -> smoltcp::time::Instant {
    let millis = Jiffies::elapsed().as_duration().as_millis();
    smoltcp::time::Instant::from_millis(millis as i64)
}
