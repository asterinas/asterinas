/// 1

use super::csr::*;

/// 1
pub fn enable_virtualization_cpu(){
    unsafe {
        Hedeleg::write(0);
        Hideleg::write(0);

        // VS should access only the time counter directly. Everything else should trap
        Hcounteren::write(0x02);

        Hvip::write(0);
    }
}

/// 1
fn disable_virtualization_cpu(){
    unsafe {
        Vsie::write(0);
        Hvip::write(0);
        Hedeleg::write(0);
        Hideleg::write(0);
    }
}
