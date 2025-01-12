use bitflags::bitflags;
use ostd::Pod;

use super::header::{VirtioGpuCtrlHdr, VirtioGpuCtrlType};


/* VIRTIO_GPU_CMD_DISPLAY_INFO */
pub(crate) const RESPONSE_SIZE: usize = size_of::<VirtioGpuRespDisplayInfo>();

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuRect {
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

impl VirtioGpuRect {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
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

impl VirtioGpuRespDisplayInfo {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
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

/* VIRTIO_GPU_CMD_GET_EDID */
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuGetEdid {
    hdr: VirtioGpuCtrlHdr,
    scanout: u32,
    padding: u32,
}

impl Default for VirtioGpuGetEdid {
    fn default() -> Self {
        VirtioGpuGetEdid {
            hdr: VirtioGpuCtrlHdr::from_type(VirtioGpuCtrlType::VIRTIO_GPU_CMD_GET_EDID),
            scanout: 0,
            padding: 0,
        }
    }
    
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespEdid {
    hdr: VirtioGpuCtrlHdr,
    size: u32,
    padding: u32,
    edid: [u8; 1024],
}
impl VirtioGpuRespEdid {
    pub(crate) fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespEdid {
    fn default() -> Self {
        VirtioGpuRespEdid {
            hdr: VirtioGpuCtrlHdr::default(),
            size: 0,
            padding: 0,
            edid: [0; 1024],
        }
    }
    
}
impl VirtioGpuRespDisplayInfo {
    pub(crate) fn get_rect(&self, index: usize) -> Option<VirtioGpuRect> {
        Some(self.pmodes[index].r)
    }
}

// VIRTIO_GPU_CMD_RESOURCE_CREATE_2D
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod, Default)]
pub struct VirtioGpuResourceCreate2D {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

impl VirtioGpuResourceCreate2D {
    pub fn new(resource_id: u32, format: VirtioGpuFormat, width: u32, height: u32) -> Self {
        VirtioGpuResourceCreate2D {
            hdr: VirtioGpuCtrlHdr::from_type(VirtioGpuCtrlType::VIRTIO_GPU_CMD_RESOURCE_CREATE_2D),
            resource_id,
            format: format as u32,
            width,
            height,
        }
    }
    
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum VirtioGpuFormat {
    VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM  = 1, 
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespResourceCreate2D {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespResourceCreate2D {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespResourceCreate2D {
    fn default() -> Self {
        VirtioGpuRespResourceCreate2D {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}