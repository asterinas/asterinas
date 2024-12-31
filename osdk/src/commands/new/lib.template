#![no_std]
#![deny(unsafe_code)]

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    #[ktest]
    fn it_works() {
        let memory_regions = &ostd::boot::boot_info().memory_regions;
        assert!(!memory_regions.is_empty());
    }
}
