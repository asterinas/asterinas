// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(parent = crate::test_override::parent)]

pub(in parent) const fn override_parent() {}
