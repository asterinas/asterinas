// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::drm::ioctl::{DrmIoctlFlags, drm_ioc, types::*},
    util::ioctl::{InData, InOutData, NoData},
};

drm_ioc!(
    DrmIoctlVersion,
    DRM_IOCTL_VERSION,
    0x00,
    InOutData<DrmVersion>,
    DrmIoctlFlags::RENDER_ALLOW
);
drm_ioc!(
    DrmIoctlGetCap,
    DRM_IOCTL_GET_CAP,
    0x0c,
    InOutData<DrmGetCap>,
    DrmIoctlFlags::RENDER_ALLOW
);
drm_ioc!(
    DrmIoctlSetClientCap,
    DRM_IOCTL_SET_CLIENT_CAP,
    0x0d,
    InData<DrmSetClientCap>,
    DrmIoctlFlags::MODESET
);
drm_ioc!(
    DrmIoctlSetMaster,
    DRM_IOCTL_SET_MASTER,
    0x1e,
    NoData,
    DrmIoctlFlags::DEFAULT
);
drm_ioc!(
    DrmIoctlDropMaster,
    DRM_IOCTL_DROP_MASTER,
    0x1f,
    NoData,
    DrmIoctlFlags::DEFAULT
);
