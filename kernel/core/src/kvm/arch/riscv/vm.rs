
use ostd::arch::vm::enable_virtualization_cpu;

/// 1
pub struct VmArch {}


impl VmArch {
    /// 1
    pub fn new() -> Self {
        Self {}
    }

    /// 2
    pub fn init(&self){
        enable_virtualization_cpu();
    }
}
