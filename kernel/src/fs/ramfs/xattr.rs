// SPDX-License-Identifier: MPL-2.0

use hashbrown::{hash_map::Entry, Equivalent, HashMap};
use spin::Once;

use crate::{
    fs::utils::{InodeType, XattrName, XattrNamespace, XattrSetFlags},
    prelude::*,
};

/// An in-memory xattr object of a `RamInode`.
/// An xattr is used to manage special 'name-value' pairs of an inode.
pub struct RamXattr(Once<Box<RwMutex<RamXattrInner>>>);

/// An owned in-memory xattr name that possesses a valid namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RamXattrName {
    namespace: XattrNamespace,
    full_name: String,
}

/// The value type of an in-memory xattr.
type RamXattrValue = Vec<u8>;

#[derive(Debug, Clone)]
struct RamXattrInner {
    map: HashMap<RamXattrName, RamXattrValue>,
    total_name_count: usize,
    total_name_len: usize,
    user_name_count: usize,
    user_name_len: usize,
}

impl RamXattr {
    pub fn new() -> Self {
        Self(Once::new())
    }

    pub fn set(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let inner = self
            .0
            .call_once(|| Box::new(RwMutex::new(RamXattrInner::new())));
        let mut xattr = inner.write();

        let namespace = name.namespace();
        let name_len = name.full_name_len();
        match xattr.map.entry(RamXattrName::from(name)) {
            Entry::Occupied(mut entry) => {
                if flags.contains(XattrSetFlags::CREATE_ONLY) {
                    return_errno_with_message!(Errno::EEXIST, "the target xattr already exists");
                }

                let value = {
                    let mut value = vec![0u8; value_reader.remain()];
                    value_reader.read_fallible(&mut VmWriter::from(value.as_mut_slice()))?;
                    value
                };
                let _ = entry.insert(value);
            }
            Entry::Vacant(entry) => {
                if flags.contains(XattrSetFlags::REPLACE_ONLY) {
                    return_errno_with_message!(Errno::ENODATA, "the target xattr does not exist");
                }

                let value = {
                    let mut value = vec![0u8; value_reader.remain()];
                    value_reader.read_fallible(&mut VmWriter::from(value.as_mut_slice()))?;
                    value
                };
                let _ = entry.insert(value);

                xattr.total_name_count += 1;
                xattr.total_name_len += name_len;
                if namespace.is_user() {
                    xattr.user_name_count += 1;
                    xattr.user_name_len += name_len;
                }
            }
        };

        Ok(())
    }

    pub fn get(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        let existence_error =
            Error::with_message(Errno::ENODATA, "the target xattr does not exist");
        let inner = self.0.get().ok_or(existence_error)?;

        let xattr = inner.read();
        if xattr.total_name_count == 0 {
            return Err(existence_error);
        }

        let value = xattr.map.get(&name).ok_or(existence_error)?;
        let value_len = value.len();

        let value_avail_len = value_writer.avail();
        if value_avail_len == 0 {
            return Ok(value_len);
        }
        if value_len > value_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr value buffer is too small");
        }

        value_writer.write_fallible(&mut VmReader::from(value.as_slice()))?;
        Ok(value_len)
    }

    pub fn list(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        let Some(inner) = self.0.get() else {
            return Ok(0);
        };
        let xattr = inner.read();

        // Include the null byte following each name
        let list_actual_len = if namespace.is_user() {
            xattr.user_name_len + xattr.user_name_count
        } else {
            xattr.total_name_len + xattr.total_name_count
        };
        let list_avail_len = list_writer.avail();
        if list_avail_len == 0 {
            return Ok(list_actual_len);
        }
        if list_actual_len > list_avail_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr list buffer is too small");
        }

        for (name, _) in &xattr.map {
            if namespace.is_user() && !name.namespace.is_user() {
                continue;
            }

            list_writer.write_fallible(&mut VmReader::from(name.full_name.as_bytes()))?;
            list_writer.write_val(&0u8)?;
        }
        Ok(list_actual_len)
    }

    pub fn remove(&self, name: XattrName) -> Result<()> {
        let existence_error =
            Error::with_message(Errno::ENODATA, "the target xattr does not exist");
        let inner = self.0.get().ok_or(existence_error)?;

        let mut xattr = inner.write();
        if xattr.total_name_count == 0 {
            return Err(existence_error);
        }

        xattr.map.remove(&name).ok_or(existence_error)?;

        let namespace = name.namespace();
        let name_len = name.full_name_len();
        xattr.total_name_count -= 1;
        xattr.total_name_len -= name_len;
        if namespace.is_user() {
            xattr.user_name_count -= 1;
            xattr.user_name_len -= name_len;
        }
        Ok(())
    }

    /// Checks if the file type is valid for xattr support.
    pub fn check_file_type_for_xattr(file_type: InodeType) -> Result<()> {
        match file_type {
            InodeType::File | InodeType::Dir => Ok(()),
            _ => Err(Error::with_message(
                Errno::EPERM,
                "xattr is not supported on the file type",
            )),
        }
    }
}

impl From<XattrName<'_>> for RamXattrName {
    fn from(value: XattrName) -> Self {
        Self {
            namespace: value.namespace(),
            full_name: value.full_name().to_string(),
        }
    }
}

impl Equivalent<RamXattrName> for XattrName<'_> {
    fn equivalent(&self, key: &RamXattrName) -> bool {
        self.namespace() == key.namespace && self.full_name() == key.full_name
    }
}

impl RamXattrInner {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            total_name_count: 0,
            total_name_len: 0,
            user_name_count: 0,
            user_name_len: 0,
        }
    }
}
