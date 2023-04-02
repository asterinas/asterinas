use crate::config::{self, PAGE_SIZE};
use limine::{LimineBootInfoRequest, LimineHhdmRequest, LimineStackSizeRequest};
use log::info;

pub fn init() {
    if let Some(bootinfo) = BOOTLOADER_INFO_REQUEST.get_response().get() {
        info!(
            "booted by {} v{}",
            bootinfo.name.to_str().unwrap().to_str().unwrap(),
            bootinfo.version.to_str().unwrap().to_str().unwrap(),
        );
    }
    let response = HHDM_REQUEST
        .get_response()
        .get()
        .expect("Not found HHDM Features");
    assert_eq!(config::PHYS_OFFSET as u64, response.offset);
    STACK_REQUEST.get_response().get().unwrap();
}

static BOOTLOADER_INFO_REQUEST: LimineBootInfoRequest = LimineBootInfoRequest::new(0);
static HHDM_REQUEST: LimineHhdmRequest = LimineHhdmRequest::new(0);
static STACK_REQUEST: LimineStackSizeRequest = {
    let a = LimineStackSizeRequest::new(0);
    // 64 * 4096(PAGE_SIZE)
    a.stack_size(64 * PAGE_SIZE as u64)
};
