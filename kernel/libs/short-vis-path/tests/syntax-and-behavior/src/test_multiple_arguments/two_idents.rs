// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(test_multiple_arguments, two_idents)]

pub(in test_multiple_arguments) type VisibleToParent = ();

pub(in two_idents) type VisibleToCurrent = ();
