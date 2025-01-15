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
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        VirtioGpuRect {
            x,
            y,
            width,
            height,
        }
    }

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

    pub(crate) fn get_rect(&self, index: usize) -> Option<VirtioGpuRect> {
        Some(self.pmodes[index].r)
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
    VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM = 1,
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

// VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING
// Assign backing pages to a resource.
// Request data is struct virtio_gpu_resource_attach_backing, followed by struct virtio_gpu_mem_entry entries.
// Response type is VIRTIO_GPU_RESP_OK_NODATA.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuResourceAttachBacking {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    nr_entries: u32,
}

impl VirtioGpuResourceAttachBacking {
    pub(crate) fn new(resource_id: u32, nr_entries: u32) -> VirtioGpuResourceAttachBacking {
        VirtioGpuResourceAttachBacking {
            hdr: VirtioGpuCtrlHdr::from_type(
                VirtioGpuCtrlType::VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING,
            ),
            resource_id,
            nr_entries,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuMemEntry {
    addr: u64,
    length: u32,
    padding: u32,
}

impl VirtioGpuMemEntry {
    pub(crate) fn new(addr: usize, length: u32) -> VirtioGpuMemEntry {
        VirtioGpuMemEntry {
            addr: addr as u64,
            length,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespAttachBacking {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespAttachBacking {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespAttachBacking {
    fn default() -> Self {
        VirtioGpuRespAttachBacking {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}

// VIRTIO_GPU_CMD_SET_SCANOUT
// Set the scanout parameters for a single output.
// Request data is struct virtio_gpu_set_scanout. Response type is VIRTIO_GPU_RESP_OK_NODATA.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuSetScanout {
    hdr: VirtioGpuCtrlHdr,
    r: VirtioGpuRect,
    scanout_id: u32,
    resource_id: u32,
}

impl VirtioGpuSetScanout {
    pub(crate) fn new(scanout_id: u32, resource_id: u32, r: VirtioGpuRect) -> VirtioGpuSetScanout {
        VirtioGpuSetScanout {
            hdr: VirtioGpuCtrlHdr::from_type(VirtioGpuCtrlType::VIRTIO_GPU_CMD_SET_SCANOUT),
            r,
            scanout_id,
            resource_id,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespSetScanout {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespSetScanout {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespSetScanout {
    fn default() -> Self {
        VirtioGpuRespSetScanout {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}

// VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D
// Transfer from guest memory to host resource.
// Request data is struct virtio_gpu_transfer_to_host_2d. Response type is VIRTIO_GPU_RESP_OK_NODATA.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuTransferToHost2D {
    hdr: VirtioGpuCtrlHdr,
    r: VirtioGpuRect,
    offset: u64,
    resource_id: u32,
    padding: u32,
}

impl VirtioGpuTransferToHost2D {
    pub(crate) fn new(
        r: VirtioGpuRect,
        offset: u64,
        resource_id: u32,
    ) -> VirtioGpuTransferToHost2D {
        VirtioGpuTransferToHost2D {
            hdr: VirtioGpuCtrlHdr::from_type(VirtioGpuCtrlType::VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D),
            r,
            offset,
            resource_id,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespTransferToHost2D {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespTransferToHost2D {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespTransferToHost2D {
    fn default() -> Self {
        VirtioGpuRespTransferToHost2D {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}

// VIRTIO_GPU_CMD_RESOURCE_FLUSH
// Flush a scanout resource.
// Request data is struct virtio_gpu_resource_flush. Response type is VIRTIO_GPU_RESP_OK_NODATA.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuResourceFlush {
    hdr: VirtioGpuCtrlHdr,
    r: VirtioGpuRect,
    resource_id: u32,
    padding: u32,
}

impl VirtioGpuResourceFlush {
    pub(crate) fn new(r: VirtioGpuRect, resource_id: u32) -> VirtioGpuResourceFlush {
        VirtioGpuResourceFlush {
            hdr: VirtioGpuCtrlHdr::from_type(VirtioGpuCtrlType::VIRTIO_GPU_CMD_RESOURCE_FLUSH),
            r,
            resource_id,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespResourceFlush {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespResourceFlush {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespResourceFlush {
    fn default() -> Self {
        VirtioGpuRespResourceFlush {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}

// VIRTIO_GPU_CMD_UPDATE_CURSOR and VIRTIO_GPU_CMD_MOVE_CURSOR
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuCursorPos {
    scanout_id: u32,
    x: u32,
    y: u32,
    padding: u32,
}

impl VirtioGpuCursorPos {
    pub(crate) fn new(scanout_id: u32, x: u32, y: u32) -> VirtioGpuCursorPos {
        VirtioGpuCursorPos {
            scanout_id,
            x,
            y,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuUpdateCursor {
    hdr: VirtioGpuCtrlHdr,
    pos: VirtioGpuCursorPos,
    resource_id: u32,
    hot_x: u32,
    hot_y: u32,
    padding: u32,
}

impl VirtioGpuUpdateCursor {
    pub(crate) fn new(
        pos: VirtioGpuCursorPos,
        resource_id: u32,
        hot_x: u32,
        hot_y: u32,
        is_move: bool,
    ) -> VirtioGpuUpdateCursor {
        let hdr_type = if is_move {
            VirtioGpuCtrlType::VIRTIO_GPU_CMD_MOVE_CURSOR
        } else {
            VirtioGpuCtrlType::VIRTIO_GPU_CMD_UPDATE_CURSOR
        };
        VirtioGpuUpdateCursor {
            hdr: VirtioGpuCtrlHdr::from_type(hdr_type),
            pos,
            resource_id,
            hot_x,
            hot_y,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespUpdateCursor {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespUpdateCursor {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespUpdateCursor {
    fn default() -> Self {
        VirtioGpuRespUpdateCursor {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}

// VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub(crate) struct VirtioGpuResourceDetachBacking {
    hdr: VirtioGpuCtrlHdr,
    resource_id: u32,
    padding: u32,
}

impl VirtioGpuResourceDetachBacking {
    pub(crate) fn new(resource_id: u32) -> VirtioGpuResourceDetachBacking {
        VirtioGpuResourceDetachBacking {
            hdr: VirtioGpuCtrlHdr::from_type(
                VirtioGpuCtrlType::VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING,
            ),
            resource_id,
            padding: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuRespDetachBacking {
    hdr: VirtioGpuCtrlHdr,
}

impl VirtioGpuRespDetachBacking {
    pub fn header_type(&self) -> u32 {
        self.hdr.type_
    }
}

impl Default for VirtioGpuRespDetachBacking {
    fn default() -> Self {
        VirtioGpuRespDetachBacking {
            hdr: VirtioGpuCtrlHdr::default(),
        }
    }
}
