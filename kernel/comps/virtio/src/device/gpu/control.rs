use bitflags::bitflags;
use ostd::Pod;
use super::header::{VirtioGpuCtrlHdr, VirtioGpuCtrlType};

pub const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VirtioGpuRect {
    x: u32, // 起始点的 X 坐标
    y: u32, // 起始点的 Y 坐标
    width: u32, // 矩形的宽度
    height: u32, // 矩形的高度
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGPUDisplayOne {
    r: VirtioGpuRect,
    // set when user enabled the display
    enable: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGPURespDisplayInfo {
    header: VirtioGpuCtrlHdr,
    pmodes: [VirtioGPUDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}
