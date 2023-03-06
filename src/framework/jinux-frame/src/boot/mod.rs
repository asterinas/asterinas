mod limine;

pub(crate) const MEMORY_MAP_MAX_COUNT: usize = 30;
pub(crate) const MODULE_MAX_COUNT: usize = 10;

/// init bootloader
pub(crate) fn init() {
    limine::init();
}
