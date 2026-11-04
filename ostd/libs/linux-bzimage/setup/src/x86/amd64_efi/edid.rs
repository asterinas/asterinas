// SPDX-License-Identifier: MPL-2.0

//! EFI display information copied before exiting boot services.
//!
//! The private protocol bindings let the EFI setup code read the EDID for its
//! selected graphics output and pass validated data through Linux boot parameters.

use linux_boot_params::EdidInfo;
use uefi::{Handle, boot, proto::Protocol};

/// Reads the active EDID, or the discovered EDID if the active protocol is unavailable.
pub(super) fn read(handle: Handle) -> Option<EdidInfo> {
    // An active protocol may intentionally override the discovered EDID with
    // no data. Do not bypass it merely because its block is empty or invalid.
    read_protocol::<EdidActive>(handle, |protocol| &protocol.0)
        .or_else(|_| read_protocol::<EdidDiscovered>(handle, |protocol| &protocol.0))
        .ok()
        .flatten()
}

// uefi 0.37 does not provide EDID bindings. Both protocols have this read-only
// layout, as specified in UEFI 2.10, sections 12.9.2.4 and 12.9.2.5:
// https://uefi.org/specs/UEFI/2.10/12_Protocols_Console_Support.html#efi-edid-discovered-protocol
#[repr(C)]
struct RawEdid {
    size_of_edid: u32,
    edid: *const u8,
}

#[repr(transparent)]
#[uefi::proto::unsafe_protocol("bd8c1056-9f36-44ec-92a8-a6337f817986")]
struct EdidActive(RawEdid);

#[repr(transparent)]
#[uefi::proto::unsafe_protocol("1c0c34f6-d380-41fa-a049-8ad06c1a66aa")]
struct EdidDiscovered(RawEdid);

fn read_protocol<P: Protocol>(
    handle: Handle,
    fields_fn: impl FnOnce(&P) -> &RawEdid,
) -> uefi::Result<Option<EdidInfo>> {
    // SAFETY:
    // 1. These private protocol types describe read-only firmware data.
    // 2. No mode changes or driver disconnections occur while the guard is alive.
    // 3. This code only reads the buffer through shared access.
    let protocol = unsafe {
        boot::open_protocol::<P>(
            boot::OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            boot::OpenProtocolAttributes::GetProtocol,
        )
    }?;
    let fields = fields_fn(&protocol);
    if fields.edid.is_null()
        || (fields.size_of_edid as usize) < linux_boot_params::EDID_BASE_BLOCK_SIZE
    {
        return Ok(None);
    }

    // SAFETY:
    // 1. UEFI provides `size_of_edid` initialized, read-only bytes; the checks
    //    above cover this fixed-size, byte-aligned read.
    // 2. No firmware calls or output/driver changes occur between reading
    //    the fields and this copy.
    // 3. The copy precedes closing the protocol and exiting boot services.
    //    No firmware pointer is retained in the boot parameters.
    let bytes = unsafe {
        fields
            .edid
            .cast::<[u8; linux_boot_params::EDID_BASE_BLOCK_SIZE]>()
            .read()
    };
    Ok(EdidInfo::from_bytes(&bytes))
}
