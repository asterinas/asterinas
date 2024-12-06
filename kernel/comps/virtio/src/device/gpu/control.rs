use bitflags::bitflags;
use ostd::Pod;

use super::header::VirtioGpuCtrlHdr;

pub(crate) const RESPONSE_SIZE: usize = size_of::<VirtioGpuRespDisplayInfo>();

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
struct VirtioGpuRect {
    /// For any coordinates given 0,0 is top left, larger x moves right, larger y moves down.
    x: u32,
    /// For any coordinates given 0,0 is top left, larger x moves right, larger y moves down.
    y: u32,
    /// similar to the native panel resolution in EDID display information,
    /// except that in the virtual machine case the size can change when the host window representing the guest display gets resized.
    width: u32,
    /// similar to the native panel resolution in EDID display information,
    /// except that in the virtual machine case the size can change when the host window representing the guest display gets resized.
    height: u32,
}

pub const VIRTIO_GPU_MAX_SCANOUTS: usize = 16;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
struct VirtioGpuDisplayOne {
    /// preferred position and size
    r: VirtioGpuRect,
    /// scanout enabled, set when the user enabled the display
    enabled: u32,
    flags: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespDisplayInfo {
    hdr: VirtioGpuCtrlHdr,
    pmodes: [VirtioGpuDisplayOne; VIRTIO_GPU_MAX_SCANOUTS],
}

impl Default for VirtioGpuRespDisplayInfo {
    fn default() -> Self {
        VirtioGpuRespDisplayInfo {
            hdr: VirtioGpuCtrlHdr::default(),
            pmodes: [VirtioGpuDisplayOne {
                r: VirtioGpuRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                enabled: 0,
                flags: 0,
            }; VIRTIO_GPU_MAX_SCANOUTS],
        }
    }
}
