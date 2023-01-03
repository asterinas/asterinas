use crate::driver::rtc::{get_cmos, is_updating, CENTURY_REGISTER, read};
use core::sync::atomic::Ordering::Relaxed;



#[derive(Debug,Clone, Copy,Default,PartialEq, Eq, PartialOrd, Ord)]
pub struct Time{
    century: u8,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Time{
    pub(crate) fn update_from_rtc(&mut self){
        while is_updating(){}
        self.second = get_cmos(0x00);
        self.minute = get_cmos(0x02);
        self.hour = get_cmos(0x04);
        self.day = get_cmos(0x07);
        self.month = get_cmos(0x08);
        self.year = get_cmos(0x09) as u16;
    
        let century_register = CENTURY_REGISTER.load(Relaxed);
    
        if century_register !=0{
            self.century = get_cmos(century_register);
        }
    }

    /// convert BCD to binary values
    /// ref:https://wiki.osdev.org/CMOS#Reading_All_RTC_Time_and_Date_Registers
    pub(crate) fn convert_bcd_to_binary(&mut self,register_b: u8){
        if register_b & 0x04 == 0{
            let century_register = CENTURY_REGISTER.load(Relaxed);
            self.second = (self.second & 0x0F) + ((self.second / 16) * 10);
            self.minute = (self.minute & 0x0F) + ((self.minute / 16) * 10);
            self.hour = ( (self.hour & 0x0F) + (((self.hour & 0x70) / 16) * 10) ) | (self.hour & 0x80);
            self.day = (self.day & 0x0F) + ((self.day / 16) * 10);
            self.month = (self.month & 0x0F) + ((self.month / 16) * 10);
            self.year = (self.year & 0x0F) + ((self.year / 16) * 10);
            if century_register != 0 {
                self.century = (self.century & 0x0F) + ((self.century / 16) * 10);
            }
        }
    }
    /// convert 12 hour clock to 24 hour clock
    pub(crate) fn convert_12_hour_to_24_hour(&mut self,register_b:u8){
        // bit1 in register_b is not set if 12 hour format is enable
        // if highest bit in hour is set, then it is pm
        if ((register_b & 0x02)==0) && ((self.hour & 0x80) !=0){
            self.hour = ((self.hour & 0x7F) + 12) % 24;
        }
    }

    /// convert raw year (10, 20 etc.) to real year (2010, 2020 etc.)
    pub(crate) fn modify_year(&mut self){
        let century_register = CENTURY_REGISTER.load(Relaxed);
        if century_register !=0{
            self.year += self.century as u16 * 100;
        }else{
            panic!("century register not exists");
        }
    }

}

/// get real time
pub fn get_real_time() -> Time{
    read()
}

