use core::mem::size_of;

use ostd::Pod;

pub(crate) const REQUEST_SIZE: usize = size_of::<VirtioGpuCtrlHdr>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)] // TODO: do we need TryFromInt?
#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum VirtioGpuCtrlType {
    /* 2d commands */
    /// Retrieve the current output configuration. No request data (just bare struct virtio_gpu_ctrl_hdr).
    ///  Response type is VIRTIO_GPU_RESP_OK_DISPLAY_INFO, response data is struct virtio_gpu_resp_display_info.
    VIRTIO_GPU_CMD_GET_DISPLAY_INFO = 0x0100,
    VIRTIO_GPU_CMD_RESOURCE_CREATE_2D,
    VIRTIO_GPU_CMD_RESOURCE_UNREF,
    VIRTIO_GPU_CMD_SET_SCANOUT,
    VIRTIO_GPU_CMD_RESOURCE_FLUSH,
    VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D,
    VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING,
    VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING,
    VIRTIO_GPU_CMD_GET_CAPSET_INFO,
    VIRTIO_GPU_CMD_GET_CAPSET,
    VIRTIO_GPU_CMD_GET_EDID,
    VIRTIO_GPU_CMD_RESOURCE_ASSIGN_UUID,
    VIRTIO_GPU_CMD_CTX_CREATE,

    /* 3d commands */
    // TODO:

    /* cursor commands */
    VIRTIO_GPU_CMD_UPDATE_CURSOR = 0x0300,
    VIRTIO_GPU_CMD_MOVE_CURSOR,

    /* success responses */
    VIRTIO_GPU_RESP_OK_NODATA = 0x1100,
    VIRTIO_GPU_RESP_OK_EDID = 0x1104,
    /* error responses */
    // TODO:
}

/// All requests and responses on the virt queues have a fixed header using the following layout structure.
/// Referece: spec 5.7.6.7 Device Operation: Request header
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct VirtioGpuCtrlHdr {
    /// specifies the type of the driver request (VIRTIO_GPU_CMD_*) or device response (VIRTIO_GPU_RESP_*).
    /// On success the device will return VIRTIO_GPU_RESP_OK_NODATA in case there is no payload. Otherwise the type field will indicate the kind of payload.
    /// On error the device will return one of the VIRTIO_GPU_RESP_ERR_* error codes.
    pub type_: u32,
    /// request / response flags.
    pub flags: u32,
    /// If the driver sets the VIRTIO_GPU_FLAG_FENCE bit in the request flags field the device MUST:
    ///  - set VIRTIO_GPU_FLAG_FENCE bit in the response,
    ///  - copy the content of the fence_id field from the request to the response, and
    ///  - send the response only after command processing is complete.
    pub fence_id: u64,
    /// Rendering context (used in 3D mode only).
    pub ctx_id: u32,
    /// ring_idx indicates the value of a context-specific ring index.
    /// For more details, refer to spec.
    pub ring_idx: u8,
    pub padding: [u8; 3],
}

impl VirtioGpuCtrlHdr {
    pub(crate) fn from_type(type_: VirtioGpuCtrlType) -> VirtioGpuCtrlHdr {
        VirtioGpuCtrlHdr {
            type_: type_ as u32,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            ring_idx: 0,
            padding: [0; 3],
        }
    }
}

impl Default for VirtioGpuCtrlHdr {
    fn default() -> Self {
        VirtioGpuCtrlHdr {
            type_: 0,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            ring_idx: 0,
            padding: [0; 3],
        }
    }
}
