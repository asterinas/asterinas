cfg_if::cfg_if! {
    if #[cfg(target_arch = "x86_64")] {
        mod amd64_efi;
    } else if #[cfg(target_arch = "x86")] {
        mod legacy_i386;
    } else {
        compile_error!("Unsupported target_arch");
    }
}

// This is enforced in the linker script of the setup.
const START_OF_SETUP32_VA: usize = 0x100000;

/// The setup is a position-independent executable. We can get the loaded base
/// address from the symbol.
#[inline]
pub fn get_image_loaded_offset() -> isize {
    extern "C" {
        fn start_of_setup32();
    }
    start_of_setup32 as isize - START_OF_SETUP32_VA as isize
}
