// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(route)]

pub(super) mod addr;
pub(super) mod link;

pub(in route) use addr::AddrAttrClass;
pub(in route) use link::LinkAttrClass;
