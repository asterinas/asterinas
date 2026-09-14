// SPDX-License-Identifier: MPL-2.0

pub(crate) mod lsm;

pub(super) fn init() {
    lsm::init();
}
