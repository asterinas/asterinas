// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::{
    sync::non_null::{ArcRef, NonNullPtr},
    util::Either,
};

use crate::node::XNode;

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
pub(super) type XEntry<P> = Either<Arc<XNode<P>>, P>;

/// A type that represents the reference to `XEntry`.
pub(super) type XEntryRef<'a, P> = Either<ArcRef<'a, XNode<P>>, <P as NonNullPtr>::Ref<'a>>;

/// A type that represents the interior entries in `XArray`.
pub(super) type NodeEntry<P> = Arc<XNode<P>>;

/// A type that represents the reference to `NodeEntry`.
pub(super) type NodeEntryRef<'a, P> = ArcRef<'a, XNode<P>>;
