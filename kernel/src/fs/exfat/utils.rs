// SPDX-License-Identifier: MPL-2.0

use core::{ops::Range, time::Duration};

use time::{OffsetDateTime, PrimitiveDateTime, Time};

use super::fat::ClusterID;
use crate::prelude::*;

pub fn make_hash_index(cluster: ClusterID, offset: u32) -> usize {
    (cluster as usize) << 32usize | (offset as usize & 0xffffffffusize)
}

pub fn calc_checksum_32(data: &[u8]) -> u32 {
    let mut checksum: u32 = 0;
    for &value in data {
        checksum = checksum.rotate_right(1).wrapping_add(value as u32);
    }
    checksum
}

/// Calculating checksum, ignoring certarin bytes in the range
pub fn calc_checksum_16(data: &[u8], ignore: core::ops::Range<usize>, prev_checksum: u16) -> u16 {
    let mut result = prev_checksum;
    for (pos, &value) in data.iter().enumerate() {
        // Ignore the checksum field
        if ignore.contains(&pos) {
            continue;
        }
        result = result.rotate_right(1).wrapping_add(value as u16);
    }
    result
}

pub fn get_value_from_range(value: u16, range: Range<usize>) -> u16 {
    (value >> range.start) & ((1 << (range.end - range.start)) - 1)
}

const DOUBLE_SECOND_RANGE: Range<usize> = 0..5;
const MINUTE_RANGE: Range<usize> = 5..11;
const HOUR_RANGE: Range<usize> = 11..16;
const DAY_RANGE: Range<usize> = 0..5;
const MONTH_RANGE: Range<usize> = 5..9;
const YEAR_RANGE: Range<usize> = 9..16;

const EXFAT_TIME_ZONE_VALID: u8 = 1 << 7;

#[derive(Default, Debug, Clone, Copy)]
pub struct DosTimestamp {
    // Timestamp at the precision of double seconds.
    pub(super) time: u16,
    pub(super) date: u16,
    // Precise time in 10ms.
    pub(super) increment_10ms: u8,
    pub(super) utc_offset: u8,
}

impl DosTimestamp {
    pub fn now() -> Result<Self> {
        #[cfg(not(ktest))]
        {
            use crate::time::clocks::RealTimeClock;
            DosTimestamp::from_duration(RealTimeClock::get().read_time())
        }

        // When ktesting, the time module has not been initialized yet, return a fake value instead.
        #[cfg(ktest)]
        {
            use crate::time::SystemTime;
            DosTimestamp::from_duration(
                SystemTime::UNIX_EPOCH.duration_since(&SystemTime::UNIX_EPOCH)?,
            )
        }
    }

    pub fn new(time: u16, date: u16, increment_10ms: u8, utc_offset: u8) -> Result<Self> {
        let time = Self {
            time,
            date,
            increment_10ms,
            utc_offset,
        };
        Ok(time)
    }

    pub fn from_duration(duration: Duration) -> Result<Self> {
        // FIXME:UTC offset information is missing.

        let date_time_result =
            OffsetDateTime::from_unix_timestamp_nanos(duration.as_nanos() as i128);
        if date_time_result.is_err() {
            return_errno_with_message!(Errno::EINVAL, "failed to parse date time.")
        }

        let date_time = date_time_result.unwrap();

        let time = ((date_time.hour() as u16) << HOUR_RANGE.start)
            | ((date_time.minute() as u16) << MINUTE_RANGE.start)
            | ((date_time.second() as u16) >> 1);
        let date = (((date_time.year() - 1980) as u16) << YEAR_RANGE.start)
            | ((date_time.month() as u16) << MONTH_RANGE.start)
            | ((date_time.day() as u16) << DAY_RANGE.start);

        const NSEC_PER_10MSEC: u32 = 10000000;
        let increment_10ms =
            (date_time.second() as u32 % 2 * 100 + date_time.nanosecond() / NSEC_PER_10MSEC) as u8;

        Ok(Self {
            time,
            date,
            increment_10ms,
            utc_offset: 0,
        })
    }

    pub fn as_duration(&self) -> Result<Duration> {
        let year = 1980 + get_value_from_range(self.date, YEAR_RANGE) as u32;
        let month_result =
            time::Month::try_from(get_value_from_range(self.date, MONTH_RANGE) as u8);
        if month_result.is_err() {
            return_errno_with_message!(Errno::EINVAL, "invalid month")
        }

        let month = month_result.unwrap();

        let day = get_value_from_range(self.date, DAY_RANGE);

        let hour = get_value_from_range(self.time, HOUR_RANGE);
        let minute = get_value_from_range(self.time, MINUTE_RANGE);
        let second = get_value_from_range(self.time, DOUBLE_SECOND_RANGE) * 2;

        let day_result = time::Date::from_calendar_date(year as i32, month, day as u8);
        if day_result.is_err() {
            return_errno_with_message!(Errno::EINVAL, "invalid day")
        }

        let time_result = Time::from_hms(hour as u8, minute as u8, second as u8);
        if time_result.is_err() {
            return_errno_with_message!(Errno::EINVAL, "invalid time")
        }

        let date_time = PrimitiveDateTime::new(day_result.unwrap(), time_result.unwrap());

        let mut sec = date_time.assume_utc().unix_timestamp() as u64;

        let mut nano_sec: u32 = 0;
        if self.increment_10ms != 0 {
            const NSEC_PER_MSEC: u32 = 1000000;
            sec += self.increment_10ms as u64 / 100;
            nano_sec = (self.increment_10ms as u32 % 100) * 10 * NSEC_PER_MSEC;
        }

        /* Adjust timezone to UTC0. */
        if (self.utc_offset & EXFAT_TIME_ZONE_VALID) != 0u8 {
            sec = Self::adjust_time_zone(sec, self.utc_offset & (!EXFAT_TIME_ZONE_VALID));
        } else {
            // TODO: Use mount info for timezone adjustment.
        }

        Ok(Duration::new(sec, nano_sec))
    }

    fn adjust_time_zone(sec: u64, time_zone: u8) -> u64 {
        if time_zone <= 0x3F {
            sec + Self::time_zone_sec(time_zone)
        } else {
            sec + Self::time_zone_sec(0x80_u8 - time_zone)
        }
    }

    fn time_zone_sec(x: u8) -> u64 {
        // Each time zone represents 15 minutes.
        x as u64 * 15 * 60
    }
}
