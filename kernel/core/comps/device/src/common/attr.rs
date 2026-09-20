// SPDX-License-Identifier: MPL-2.0

//! Device attributes exposed as files in sysfs.
//!
//! [`Attr<D>`] declares an attribute's name, permissions, and read or write callbacks.
//! The callbacks receive a reference to the device as `&D`.
//!
//! [`AttrTable`] collects the attributes of one device
//! and dispatches file reads and writes to their callbacks.
//! It stores [`TyErasedAttr`] entries whose callbacks accept `&dyn AnyDevice`,
//! allowing the table to serve devices of different Rust types.
//! Driver attributes are added when the device is bound and removed when it is unbound.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use core::fmt::{self, Write};

use aster_systree::{MAX_ATTR_SIZE, SysAttrSet, SysAttrSetBuilder, SysPerms};
use aster_util::printer::VmPrinter;
use ostd::{
    mm::{FallibleVmRead, VmReader, VmWriter},
    sync::RwMutex,
};

use crate::{
    bus::{Bus, BusDevice, DriverHandle},
    common::{AnyDevice, Error, Result, SysStr},
};

/// A function that produces the text of an attribute.
pub type ShowFn<D> = fn(&D, &mut dyn Write) -> Result<()>;

/// A function that consumes the text written to an attribute.
pub type StoreFn<D> = fn(&D, &str) -> Result<()>;

/// A statically declared attribute of devices of type `D`.
pub struct Attr<D: ?Sized> {
    name: &'static str,
    perms: SysPerms,
    show: Option<ShowFn<D>>,
    store: Option<StoreFn<D>>,
}

impl<D: ?Sized> Attr<D> {
    /// Creates a read-only attribute.
    pub const fn ro(name: &'static str, show: ShowFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RO_ATTR_PERMS,
            show: Some(show),
            store: None,
        }
    }

    /// Creates a read-write attribute.
    pub const fn rw(name: &'static str, show: ShowFn<D>, store: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::DEFAULT_RW_ATTR_PERMS,
            show: Some(show),
            store: Some(store),
        }
    }

    /// Creates a write-only attribute.
    pub const fn wo(name: &'static str, store: StoreFn<D>) -> Self {
        Self {
            name,
            perms: SysPerms::OWNER_W,
            show: None,
            store: Some(store),
        }
    }

    /// Returns the name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the permissions.
    pub fn perms(&self) -> SysPerms {
        self.perms
    }
}

impl<D: ?Sized> Clone for Attr<D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: ?Sized> Copy for Attr<D> {}

impl<D: ?Sized> fmt::Debug for Attr<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

/// An attribute whose callbacks take `&dyn AnyDevice`.
#[derive(Clone)]
pub struct TyErasedAttr {
    name: &'static str,
    perms: SysPerms,
    show: Option<TyErasedShowFn>,
    store: Option<TyErasedStoreFn>,
}

impl fmt::Debug for TyErasedAttr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TyErasedAttr")
            .field("name", &self.name)
            .field("perms", &self.perms)
            .finish()
    }
}

impl TyErasedAttr {
    /// Converts a slice of typed attributes into type-erased attributes.
    pub(crate) fn from_typed_slice<D: AnyDevice>(attrs: &[Attr<D>]) -> Vec<Self> {
        attrs.iter().map(Self::from_typed).collect()
    }

    /// Converts an attribute for devices of Rust type `D` into a type-erased attribute.
    pub(crate) fn from_typed<D: AnyDevice>(attr: &Attr<D>) -> Self {
        let show = attr.show.map(|show| -> TyErasedShowFn {
            Arc::new(move |dev: &dyn AnyDevice, w: &mut dyn Write| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                show(dev, w)
            })
        });
        let store = attr.store.map(|store| -> TyErasedStoreFn {
            Arc::new(move |dev: &dyn AnyDevice, s: &str| {
                let dev = dev.as_any().downcast_ref::<D>().unwrap();
                store(dev, s)
            })
        });
        Self {
            name: attr.name,
            perms: attr.perms,
            show,
            store,
        }
    }

    /// Converts a driver's attributes into type-erased attributes.
    ///
    /// Their callbacks hold the device's binding lock
    /// and check that the device is still bound to the same driver before running.
    pub(crate) fn from_driver<B: Bus>(driver: &Arc<DriverHandle<B>>) -> Vec<Self> {
        let mut attrs = Vec::new();
        for attr in driver.dev_attrs() {
            let show = attr.show.map(|show| -> TyErasedShowFn {
                let owner = Arc::downgrade(driver);
                Arc::new(move |dev, w| {
                    let dev = dev.as_any().downcast_ref::<BusDevice<B>>().unwrap();
                    let _guard = dev.bind_lock().lock();
                    let driver = dev.driver().ok_or(Error::NotBound)?;
                    if !Weak::ptr_eq(&owner, &Arc::downgrade(&driver)) {
                        return Err(Error::NotBound);
                    }
                    show(dev, w)
                })
            });
            let store = attr.store.map(|store| -> TyErasedStoreFn {
                let owner = Arc::downgrade(driver);
                Arc::new(move |dev, s| {
                    let dev = dev.as_any().downcast_ref::<BusDevice<B>>().unwrap();
                    let _guard = dev.bind_lock().lock();
                    let driver = dev.driver().ok_or(Error::NotBound)?;
                    if !Weak::ptr_eq(&owner, &Arc::downgrade(&driver)) {
                        return Err(Error::NotBound);
                    }
                    store(dev, s)
                })
            });
            attrs.push(Self {
                name: attr.name,
                perms: attr.perms,
                show,
                store,
            });
        }
        attrs
    }

    /// Wraps an attribute that already operates on `&dyn AnyDevice`.
    pub(crate) fn from_dyn(attr: &Attr<dyn AnyDevice>) -> Self {
        Self {
            name: attr.name,
            perms: attr.perms,
            show: attr.show.map(|show| -> TyErasedShowFn { Arc::new(show) }),
            store: attr
                .store
                .map(|store| -> TyErasedStoreFn { Arc::new(store) }),
        }
    }
}

/// The attributes of one device:
/// the `SysTree` attribute set that sysfs lists, plus the callbacks behind each entry.
///
/// A device's attributes arrive in layers and a driver's go away again,
/// so the table changes over the device's life.
/// What it publishes to `SysTree` does not: a [`SysAttrSet`] is immutable,
/// so every change builds a whole new set from the old one and swaps it in.
/// [`SysAttrSetBuilder::from_set`] carries the surviving attributes across with the IDs they had,
/// which is what keeps a file's inode number stable when a driver binds beside it.
///
/// The set and the callbacks live under one lock,
/// so the two never disagree about which attributes exist.
#[derive(Debug)]
pub(crate) struct AttrTable {
    inner: RwMutex<TableInner>,
}

impl AttrTable {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwMutex::new(TableInner {
                set: SysAttrSet::empty().clone(),
                ops: BTreeMap::new(),
            }),
        }
    }

    /// Returns the attribute set, in the form sysfs consumes.
    pub(crate) fn set(&self) -> Arc<SysAttrSet> {
        self.inner.read().set.clone()
    }

    /// Adds attributes.
    /// Fails if any name is invalid, is already present, or is repeated within `attrs`,
    /// or if the ID space is exhausted; in every case none of the attributes is added.
    pub(crate) fn add(&self, attrs: Vec<TyErasedAttr>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut seen = BTreeSet::new();
        if attrs
            .iter()
            .any(|attr| inner.ops.contains_key(attr.name) || !seen.insert(attr.name))
        {
            return Err(Error::NameConflict);
        }

        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for attr in &attrs {
            builder.add(SysStr::from(attr.name), attr.perms);
        }
        // The new set is built to one side, so an exhausted ID space is
        // reported here with the table still exactly as it was.
        inner.set = Arc::new(builder.build()?);
        for attr in attrs {
            inner.ops.insert(SysStr::from(attr.name), attr);
        }
        Ok(())
    }

    /// Removes attributes by name.
    /// Names that are absent are ignored.
    pub(crate) fn remove(&self, names: &[&'static str]) {
        let mut inner = self.inner.write();
        let mut builder = SysAttrSetBuilder::from_set(&inner.set);
        for name in names {
            inner.ops.remove(*name);
            builder.remove(name);
        }
        let set = builder
            .build()
            .expect("a builder that only removes attributes cannot fail");
        inner.set = Arc::new(set);
    }

    /// Reads attribute `name` for `dev` into `writer`.
    ///
    /// Skips the first `offset` bytes of the attribute's text
    /// and returns the number of bytes written.
    pub(crate) fn show(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        let show = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.show
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let mut printer = VmPrinter::new_skip(writer, offset);
        show(dev, &mut printer)?;
        Ok(printer.bytes_written())
    }

    /// Writes text from `reader` to attribute `name` for `dev`.
    ///
    /// Reads at most `MAX_ATTR_SIZE` bytes and passes the text to the attribute's write callback.
    /// Returns the number of bytes consumed.
    pub(crate) fn store(
        &self,
        dev: &dyn AnyDevice,
        name: &str,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        let store = {
            let inner = self.inner.read();
            let attr = inner.ops.get(name).ok_or(aster_systree::Error::NotFound)?;
            attr.store
                .clone()
                .ok_or(aster_systree::Error::PermissionDenied)?
        };
        let (text, len) = read_text(reader)?;
        store(dev, &text)?;
        Ok(len)
    }
}

/// Reads UTF-8 text from `reader`.
///
/// Consumes at most `MAX_ATTR_SIZE` bytes
/// and returns the text and the number of bytes consumed.
pub(crate) fn read_text(reader: &mut VmReader) -> aster_systree::Result<(String, usize)> {
    let mut buf = vec![0u8; MAX_ATTR_SIZE];
    let mut writer = VmWriter::from(buf.as_mut_slice());
    let len = reader
        .read_fallible(&mut writer)
        .map_err(|_| aster_systree::Error::PageFault)?;
    let text =
        core::str::from_utf8(&buf[..len]).map_err(|_| aster_systree::Error::InvalidOperation)?;
    Ok((String::from(text), len))
}

#[derive(Debug)]
struct TableInner {
    /// What sysfs lists.
    /// Replaced wholesale, never edited.
    set: Arc<SysAttrSet>,
    /// What serves a read or a write of each listed attribute.
    ops: BTreeMap<SysStr, TyErasedAttr>,
}

type TyErasedShowFn = Arc<dyn Fn(&dyn AnyDevice, &mut dyn Write) -> Result<()> + Send + Sync>;
type TyErasedStoreFn = Arc<dyn Fn(&dyn AnyDevice, &str) -> Result<()> + Send + Sync>;
