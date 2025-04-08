// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{marker::PhantomData, mem::ManuallyDrop, num::NonZero, ops::Deref, ptr::NonNull};

use super::node::XNode;
use crate::sync::non_null::{ArcRef, NonNullPtr};

#[derive(PartialEq)]
pub struct XEntryRef<'a, P>
where
    P: NonNullPtr + Sync,
{
    inner: ManuallyDrop<XEntry<P>>,
    _marker: PhantomData<&'a XEntry<P>>,
}

/// The type represents the internal entries in `XArray`.
pub type NodeEntry<P> = Arc<XNode<P>>;
/// The type represents the reference to `NodeEntry`.
pub type NodeEntryRef<'a, P> = ArcRef<'a, XNode<P>>;

impl<P> Deref for XEntryRef<'_, P>
where
    P: NonNullPtr + Sync,
{
    type Target = XEntry<P>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a, P> XEntryRef<'a, P>
where
    P: NonNullPtr + Sync,
{
    pub(super) fn as_node_ref(&self) -> Option<NodeEntryRef<'a, P>> {
        if !self.is_node() {
            return None;
        }

        // SAFETY:
        // - The pointer is owned by `XEntry` and will outlives the lifetime
        //   of `'a`.
        // - There is no mutable references to the pointer.
        unsafe { Some(NodeEntry::raw_as_ref(self.ptr())) }
    }

    pub(super) fn as_item_ref(&self) -> Option<P::Ref<'a>> {
        if !self.is_item() {
            return None;
        }

        // SAFETY:
        // - The pointer is owned by `XEntry` and will outlives the lifetime
        //   of `'a`.
        // - There is no mutable references to the pointer.
        unsafe { Some(P::raw_as_ref(self.ptr())) }
    }
}

unsafe impl<P> NonNullPtr for XEntry<P>
where
    P: NonNullPtr + Sync,
{
    type Ref<'a>
        = XEntryRef<'a, P>
    where
        Self: 'a;

    fn into_raw(self) -> core::ptr::NonNull<()> {
        let ptr = ManuallyDrop::new(self);
        ptr.raw
    }

    unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        Self {
            raw: ptr,
            _marker: PhantomData,
        }
    }

    unsafe fn raw_as_ref<'a>(raw: NonNull<()>) -> Self::Ref<'a> {
        let entry = XEntry {
            raw,
            _marker: PhantomData,
        };

        XEntryRef {
            inner: ManuallyDrop::new(entry),
            _marker: PhantomData,
        }
    }

    fn ref_as_raw(ptr_ref: Self::Ref<'_>) -> NonNull<()> {
        ManuallyDrop::into_inner(ptr_ref.inner).into_raw()
    }
}

/// A type serving as the basic unit of storage for `XArray`s, used in the head of the `XArray` and
/// the slots of `XNode`s.
///
/// There are the following types of `XEntry`:
/// - Internal entries: These are invisible to users and have the last two bits set to 00.
/// - Item entries: These represent user-given items and have the last two bits set to 10.
///
/// An `XEntry` owns the item or node that it represents. Once an `XEntry` generated from an item
/// or an `XNode`, the ownership of the item or the `XNode` will be transferred to the `XEntry`.
#[derive(PartialEq, Eq)]
#[repr(transparent)]
pub struct XEntry<P>
where
    P: NonNullPtr + Sync,
{
    raw: NonNull<()>,
    _marker: core::marker::PhantomData<(Arc<XNode<P>>, P)>,
}

// SAFETY: `XEntry<P>` represents a value of either `Arc<XNode<P>>` or `P`.
unsafe impl<P: NonNullPtr + Sync> Send for XEntry<P> {}
unsafe impl<P: NonNullPtr + Sync> Sync for XEntry<P> {}

#[derive(PartialEq, Eq, Debug)]
#[repr(usize)]
enum EntryType {
    Node = 0,
    Item = 2,
}

impl TryFrom<usize> for EntryType {
    type Error = ();

    fn try_from(val: usize) -> Result<Self, Self::Error> {
        match val {
            x if x == EntryType::Node as usize => Ok(EntryType::Node),
            x if x == EntryType::Item as usize => Ok(EntryType::Item),
            _ => Err(()),
        }
    }
}

impl<P: NonNullPtr + Sync> XEntry<P> {
    const TYPE_MASK: usize = 0b11;

    /// Creates a new `XEntry`.
    ///
    ///  # Safety
    ///
    /// `ptr` must be returned from `Arc::<XNode<P>>::into_raw` or `P::into_raw` and be
    /// consistent with `ty`. In addition, the ownership of the value of `Arc<XNode<P>>`
    /// or `I` must be transferred to the constructed instance of `XEntry`.
    unsafe fn new(ptr: NonNull<()>, ty: EntryType) -> Self {
        let raw = ptr.map_addr(|addr| {
            debug_assert!(addr.get() & Self::TYPE_MASK == 0);
            addr | (ty as usize)
        });
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    fn ptr(&self) -> NonNull<()> {
        self.raw
            .map_addr(|addr| unsafe { NonZero::new_unchecked(addr.get() & !Self::TYPE_MASK) })
    }

    fn ty(&self) -> EntryType {
        (self.raw.addr().get() & Self::TYPE_MASK)
            .try_into()
            .unwrap()
    }
}

impl<P: NonNullPtr + Sync> XEntry<P> {
    pub fn from_item(item: P) -> Self {
        let item_ptr = P::into_raw(item);
        // SAFETY: `item_ptr` is returned from `P::from_raw` and the ownership of the value of `I`
        // is transferred.
        unsafe { Self::new(item_ptr, EntryType::Item) }
    }

    pub fn is_item(&self) -> bool {
        self.ty() == EntryType::Item
    }
}

impl<P: NonNullPtr + Sync> XEntry<P> {
    pub fn from_node(node: XNode<P>) -> Self {
        let node_ptr = {
            let node = Arc::new(node);
            NonNullPtr::into_raw(node)
        };
        // SAFETY: `node_ptr` is returned from `Arc::<Node<P>>::into_raw` and the ownership of the
        // value of `Arc<XNode<I, Slot>>` is transferred.
        unsafe { Self::new(node_ptr, EntryType::Node) }
    }

    pub fn is_node(&self) -> bool {
        self.ty() == EntryType::Node
    }
}

impl<P: NonNullPtr + Sync> Drop for XEntry<P> {
    fn drop(&mut self) {
        match self.ty() {
            // SAFETY: `self` owns the value of `I`.
            EntryType::Item => unsafe {
                P::from_raw(self.ptr());
            },
            // SAFETY: `self` owns the value of `Arc<XNode<P>>`.
            EntryType::Node => unsafe {
                <NodeEntry<P> as NonNullPtr>::from_raw(self.ptr());
            },
        }
    }
}
