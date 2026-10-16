// SPDX-License-Identifier: MPL-2.0

use crate::event::DrmEventBase;

/// `struct drm_event_vblank` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L1369-L1376>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmVblankResponse {
    base: DrmEventBase,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    crtc_id: u32,
}
