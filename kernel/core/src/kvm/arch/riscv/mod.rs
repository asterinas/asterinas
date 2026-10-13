
pub mod vcpu;
pub mod vm;
pub mod ioctl;
pub mod mmio;

#[derive(PartialEq)]
enum KvmExitReturn {
    ExitToUserSpace,
	ContinueRun,
}






