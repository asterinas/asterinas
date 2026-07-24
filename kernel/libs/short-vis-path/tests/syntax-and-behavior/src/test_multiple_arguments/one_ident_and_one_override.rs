// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(test_multiple_arguments, parent = crate::test_multiple_arguments)]

pub(in parent) type VisibleToParent = ();

pub(in test_multiple_arguments) type VisibleToParentToo = ();
