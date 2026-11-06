// SPDX-License-Identifier: MPL-2.0

use crate::kms::display_mode::DrmDisplayMode;

#[derive(Clone, Debug)]
pub(crate) struct DrmCrtcState {
    enable_state: CrtcEnableState,
    // TODO: Add CRTC properties, such as color-management state.
}

impl Default for DrmCrtcState {
    fn default() -> Self {
        Self {
            enable_state: CrtcEnableState::Disabled,
        }
    }
}

impl DrmCrtcState {
    pub(crate) fn new(display_mode: DrmDisplayMode) -> Self {
        Self {
            enable_state: CrtcEnableState::Enabled {
                display_mode,
                is_active: true,
            },
        }
    }

    pub(crate) fn display_mode(&self) -> Option<DrmDisplayMode> {
        match self.enable_state {
            CrtcEnableState::Disabled => None,
            CrtcEnableState::Enabled { display_mode, .. } => Some(display_mode),
        }
    }
}

/// The CRTC's enable state, requiring a mode when enabled and preventing activity when disabled.
#[derive(Clone, Debug)]
enum CrtcEnableState {
    Disabled,
    Enabled {
        display_mode: DrmDisplayMode,
        #[expect(unused)]
        is_active: bool,
    },
}
