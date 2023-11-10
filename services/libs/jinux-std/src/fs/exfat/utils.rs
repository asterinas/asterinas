use core::time::Duration;
use crate::prelude::*;
use super::constants::EXFAT_TZ_VALID;


#[cfg(target_arch = "x86_64")]
pub fn le16_to_cpu(a:u16) -> u16{
    a
}

#[cfg(target_arch = "x86_64")]
pub fn le32_to_cpu(a:u32) -> u32{
    a
}

#[cfg(target_arch = "x86_64")]
pub fn le64_to_cpu(a:u64) -> u64{
    a
}

//time_cs has the unit of 10ms,from 0~1990ms.
pub fn convert_dos_time_to_duration(time_zone:u8,date:u16,time:u16,time_cs:u8) -> Result<core::time::Duration>{
    let year = 1980 + (date >> 9) as u32;
    let month_result = time::Month::try_from(((date >>5) & 0x000F) as u8);
    if month_result.is_err() {
        return_errno!(Errno::EINVAL)
    }

    let month = month_result.unwrap();

    let day = date & 0x001F;

    let hour = time >> 11;
    let minute = (time>>5) * 0x003F;
    let second = (time & 0x001F) << 1;

    let day_result = time::Date::from_calendar_date(year as i32, month, day as u8);
    if day_result.is_err() {
        return_errno!(Errno::EINVAL)
    }

    //FIXME: Should use unix date
    let mut sec = day_result.unwrap().to_julian_day() as u64 * 24 * 3600 + hour as u64 * 3600 + minute as u64 * 60 + second as u64;    
    let mut nano_sec:u32 = 0;
    if time_cs != 0 {
        const NSEC_PER_MSEC : u32 = 1000000;
        sec += time_cs as u64 / 100;
        nano_sec = (time_cs as u32 %100 ) * 10 * NSEC_PER_MSEC;
    }

    /* Adjust timezone to UTC0. */
    if (time_zone & EXFAT_TZ_VALID) != 0u8 {
        sec = ajust_time_zone(sec, time_zone & (!EXFAT_TZ_VALID));
    } else {
        //TODO: Use mount info for timezone adjustment.
    }

    Ok(Duration::new(sec, nano_sec))
}


pub fn convert_duration_to_dos_time(duration: Duration) -> (u8,u16,u16,u8) {
    unimplemented!();

    // let sec = duration.as_secs();
    // let nano_sec = duration.subsec_nanos();

    // let time:u16;
    // let date:u16;
    // let time_cs:u8;

    // (EXFAT_TZ_VALID,time,date,time_cs)
}

fn ajust_time_zone(sec:u64,time_zone:u8) -> u64 {
    if time_zone <= 0x3F {
        sec + time_zone_sec(time_zone)
    } else {
        sec + time_zone_sec(0x80 as u8 - time_zone)
    }
}

fn time_zone_sec(x:u8)->u64{
    //Each time zone represents 15 minutes.
    x as u64 * 15 * 60
}