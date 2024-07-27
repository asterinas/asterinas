// SPDX-License-Identifier: MPL-2.0

pub(super) use abs::{
    alloc_ephemeral_abstract_name, create_abstract_name, lookup_abstract_name, AbstractHandle,
};
pub(super) use path::{create_socket_file, lookup_socket_file};

mod abs;
mod path;
