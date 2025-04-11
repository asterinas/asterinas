// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::node::XNode;
use crate::{
    sync::non_null::{ArcRef, NonNullPtr},
    util::either::Either,
};

/// A type serving as the basic unit of storage for `XArray`s, used in the head of the `XArray` and
/// the slots of `XNode`s.
///
/// There are the following types of `XEntry`:
/// - Internal entries: These are invisible to users. Currently these entries represent pointers to
///   `XNode`s (`Arc<XNode<P>>`).
/// - Item entries: These represent user-given items of type `P`.
///
/// An `XEntry` owns the item or node that it represents. Once an `XEntry` generated from an item
/// or an `XNode`, the ownership of the item or the `XNode` will be transferred to the `XEntry`.
pub type XEntry<P> = Either<Arc<XNode<P>>, P>;
/// The type represents the reference to `XEntry`.
pub type XEntryRef<'a, P> = Either<ArcRef<'a, XNode<P>>, <P as NonNullPtr>::Ref<'a>>;
/// The type represents the internal entries in `XArray`.
pub type NodeEntry<P> = Arc<XNode<P>>;
/// The type represents the reference to `NodeEntry`.
pub type NodeEntryRef<'a, P> = ArcRef<'a, XNode<P>>;
