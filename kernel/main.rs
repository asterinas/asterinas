#![no_std]
#![no_main]
// The `export_name` attribute for the `jinux_main` entrypoint requires the removal of safety check.
// Please be aware that the kernel is not allowed to introduce any other unsafe operations.
// #![forbid(unsafe_code)]
extern crate jinux_frame;

use jinux_frame::early_println;

#[export_name = "jinux_main"]
pub fn main() -> ! {
    jinux_frame::init();
    early_println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    jinux_std::init();
    jinux_std::run_first_process();
}
