// SPDX-License-Identifier: MPL-2.0

//! Enabled-time accounting during thread scheduling intervals.

/// Enabled time on the target task's scheduled context, excluding sleep.
#[derive(Debug, Default)]
pub(super) struct ActiveTime {
    total: u64,
    since: Option<u64>,
}

impl ActiveTime {
    pub(super) fn total(&self) -> u64 {
        self.total
    }
    pub(super) fn update(&mut self, now: u64, active: bool) -> Option<()> {
        if let Some(since) = self.since {
            self.total = self.total.checked_add(now.checked_sub(since)?)?;
        }
        self.since = active.then_some(now);
        Some(())
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;

    #[ktest]
    fn enabled_time_excludes_sleep_and_disabled_time() {
        let mut time = ActiveTime::default();
        time.update(100, true).unwrap();
        time.update(120, false).unwrap();
        time.update(1000, false).unwrap(); // Sleeping or disabled.
        assert_eq!(time.total(), 20);
        time.update(2000, true).unwrap();
        time.update(2010, true).unwrap(); // Sampling does not double count.
        time.update(2020, false).unwrap();
        assert_eq!(time.total(), 40);
    }

    #[ktest]
    fn time_overflow_and_backward_clock_do_not_saturate_silently() {
        let mut time = ActiveTime {
            total: u64::MAX,
            since: Some(10),
        };
        assert!(time.update(11, true).is_none());
        assert_eq!(time.total(), u64::MAX);
        let mut time = ActiveTime::default();
        time.update(10, true).unwrap();
        assert!(time.update(9, false).is_none());
        assert_eq!(time.total(), 0);
    }
}
