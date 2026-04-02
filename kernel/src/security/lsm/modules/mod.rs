// SPDX-License-Identifier: MPL-2.0

pub(crate) mod aster;
pub(crate) mod yama;

use super::LsmModule;

static ACTIVE_MODULES: [&dyn LsmModule; 2] = [&aster::ASTER_LSM, &yama::YAMA_LSM];

pub(super) fn active_modules() -> &'static [&'static dyn LsmModule] {
    &ACTIVE_MODULES
}
