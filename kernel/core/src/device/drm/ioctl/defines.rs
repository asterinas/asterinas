// SPDX-License-Identifier: MPL-2.0

use crate::{
    device::drm::ioctl::{DrmIoctlFlags, drm_ioc, types::*},
    util::ioctl::{InData, InOutData, NoData, OutData},
};

drm_ioc!(
    DrmIoctlVersion,
    DRM_IOCTL_VERSION,
    0x00,
    InOutData<DrmVersion>,
    DrmIoctlFlags::RENDER_ALLOW
);
drm_ioc!(
    DrmIoctlGetUnique,
    DRM_IOCTL_GET_UNIQUE,
    0x01,
    InOutData<DrmUnique>,
    DrmIoctlFlags::DEFAULT
);
drm_ioc!(
    DrmIoctlGetMagic,
    DRM_IOCTL_GET_MAGIC,
    0x02,
    OutData<DrmAuth>,
    DrmIoctlFlags::DEFAULT
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
    DrmIoctlAuthMagic,
    DRM_IOCTL_AUTH_MAGIC,
    0x11,
    InData<DrmAuth>,
    DrmIoctlFlags::MASTER
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
