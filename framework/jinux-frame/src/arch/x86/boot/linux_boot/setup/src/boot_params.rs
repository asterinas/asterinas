const FIELD_PAYLOAD_OFFSET: u32 = 0x248;
const FIELD_PAYLOAD_LENGTH: u32 = 0x24c;

/// Safty: user must ensure that the boot_params_ptr is valid
pub unsafe fn get_payload_offset(boot_params_ptr: u32) -> u32 {
    *((boot_params_ptr + FIELD_PAYLOAD_OFFSET) as *const u32)
}

/// Safty: user must ensure that the boot_params_ptr is valid
pub unsafe fn get_payload_length(boot_params_ptr: u32) -> u32 {
    *((boot_params_ptr + FIELD_PAYLOAD_LENGTH) as *const u32)
}
