use jinux_pci::{capability::vendor::virtio::CapabilityVirtioData, util::BAR};
use pod::Pod;
use jinux_util::frame_ptr::InFramePtr;
use bitflags::bitflags;

bitflags!{
    /// Vsock feature bits since v1.2
    /// If no feature bit is set, only stream socket type is supported. 
    /// If VIRTIO_VSOCK_F_SEQPACKET has been negotiated, the device MAY act as if VIRTIO_VSOCK_F_STREAM has also been negotiated.
    pub struct VsockFeatures: u64 {
        const VIRTIO_VSOCK_F_STREAM = 1 << 0; // stream socket type is supported.
        const VIRTIO_VSOCK_F_SEQPACKET = 1 << 1; //seqpacket socket type is supported.
    }
}

impl VsockFeatures {
    pub fn support_features() -> Self {
        VsockFeatures::VIRTIO_VSOCK_F_STREAM
    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct VirtioVsockConfig{
    /// The guest_cid field contains the guest’s context ID, which uniquely identifies
    /// the device for its lifetime. The upper 32 bits of the CID are reserved and zeroed.
    ///
    /// According to virtio spec v1.1 2.4.1 Driver Requirements: Device Configuration Space,
    /// drivers MUST NOT assume reads from fields greater than 32 bits wide are atomic.
    /// So we need to split the u64 guest_cid into two parts.
    // read only
    pub guest_cid_low: u32,
    // read only
    pub guest_cid_high: u32,
}

impl VirtioVsockConfig {
    pub(crate) fn new(cap: &CapabilityVirtioData, bars: [Option<BAR>; 6]) -> InFramePtr<Self> {
        let bar = cap.bar;
        let offset = cap.offset;
        match bars[bar as usize].expect("Virtio pci sock cfg:bar is none") {
            BAR::Memory(address, _, _, _) => InFramePtr::new(address as usize + offset as usize)
                .expect("can not get in frame ptr for virtio socket config"),
            BAR::IO(_, _) => panic!("Virtio pci sock cfg:bar is IO type"),
        }
    }
}