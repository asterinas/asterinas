// SPDX-License-Identifier: MPL-2.0

//! The real-object reference model beneath the overlay namespace.
//!
//! One [`RealObject`] is one underlying filesystem entry as seen from one layer: the mount-level
//! layer index it carries and the real dentry that anchors it. [`RealObject::new_upper`] builds
//! the upper's object and pins the index to `UPPER_LAYER_INDEX`, and [`RealObject::new_lower`]
//! takes the index the layer stack assigned, so the side a value stands on is carried by the value
//! and by its enumerated construction points.
//!
//! The type offers two exits to the real side: [`RealObject::real_inode`] hands out the real inode
//! of the anchored dentry, and [`RealObject::dentry`] hands out that dentry itself, which some
//! real-inode writes take as their anchor. Neither exit is side-restricted: the read take point
//! `OverlayInode::real_object` also hands out lower objects, and the type does not stop a caller
//! from writing through one. Review visibility of the write paths is the only thing keeping those
//! writes honest, so a new write site must issue from a take point that has already promoted the
//! object.
//!
//! Real-directory enumeration lives here: it walks only the real side and holds no overlay state.
//!
//! Anchor validity follows the overlay lifetime: the owning layer strongly
//! holds its root dentry together with the filesystem it is rooted on, so a
//! reachable logical object never observes a dead anchor.

use core::cmp::min;

use crate::{
    fs::{
        file::{InodeType, StatusFlags},
        utils::DirentVisitor,
        vfs::{
            inode::Inode,
            path::{self, Dentry},
        },
    },
    prelude::*,
};

/// The layer index every upper real object carries: layer 0 is reserved for the upper.
const UPPER_LAYER_INDEX: usize = 0;

/// One underlying filesystem entry as seen from one layer: its layer index and dentry.
#[derive(Debug)]
pub(super) struct RealObject {
    layer_index: usize,
    dentry: Arc<Dentry>,
}

impl RealObject {
    /// Returns this real object's mount-level layer index.
    pub(super) fn layer_index(&self) -> usize {
        self.layer_index
    }

    pub(super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    pub(super) fn real_inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }

    /// Returns the status flags a read of this real object carries.
    ///
    /// A lower read takes `O_NOATIME`, so it never updates the lower's atime.
    pub(super) fn status_flags_for_read(&self, status_flags: StatusFlags) -> StatusFlags {
        if self.layer_index == UPPER_LAYER_INDEX {
            status_flags
        } else {
            status_flags | StatusFlags::O_NOATIME
        }
    }

    /// Builds the upper-side object of one upper entry; the upper is layer 0 by construction.
    pub(super) fn new_upper(dentry: Arc<Dentry>) -> Self {
        Self {
            layer_index: UPPER_LAYER_INDEX,
            dentry,
        }
    }

    /// Builds the lower-side object; `layer_index` is the mount-level index the stack assigned.
    pub(super) fn new_lower(layer_index: usize, dentry: Arc<Dentry>) -> Self {
        debug_assert!(layer_index != UPPER_LAYER_INDEX);
        Self {
            layer_index,
            dentry,
        }
    }

    /// Writes `self`'s owner, group, mode, and (regular files) size onto `temp_dentry`.
    ///
    /// A symlink's mode is left alone, as in Linux `ovl_set_attr`.
    ///
    /// Reference:
    /// <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/copy_up.c#L392-L416>.
    pub(super) fn copy_metadata_to(&self, temp_dentry: &Dentry) -> Result<()> {
        let source_inode = self.real_inode();
        let temp_inode = temp_dentry.inode();
        temp_inode.set_owner(temp_dentry, source_inode.owner()?)?;
        temp_inode.set_group(temp_dentry, source_inode.group()?)?;
        if !matches!(source_inode.type_(), InodeType::SymLink) {
            temp_inode.set_mode(temp_dentry, source_inode.mode()?)?;
        }
        if source_inode.type_().is_regular_file() {
            temp_inode.resize(temp_dentry, source_inode.size())?;
        }
        Ok(())
    }

    /// Copies `self`'s regular-file bytes into `temp_dentry`'s object; a short copy is `EIO`, while
    /// a short read advances the loop.
    pub(super) fn copy_data_to(&self, temp_dentry: &Dentry) -> Result<()> {
        /// The bytes one copy-up read moves; a short read only advances the loop.
        const COPY_CHUNK_SIZE: usize = 64 * 1024;
        let size = self.real_inode().size();
        let mut offset = 0usize;
        let mut buffer = vec![0u8; COPY_CHUNK_SIZE];
        while offset < size {
            let chunk = min(COPY_CHUNK_SIZE, size - offset);
            let mut writer = VmWriter::from(&mut buffer[..chunk]).to_fallible();
            let read_len = self
                .real_inode()
                .read_at(offset, &mut writer, StatusFlags::empty())?;
            // A zero-length read before the declared size ends the copy; the completeness check
            // below reports it, because a short copy is the failure this loop must not hide.
            if read_len == 0 {
                break;
            }
            let mut reader = VmReader::from(&buffer[..read_len]).to_fallible();
            let write_len =
                temp_dentry
                    .inode()
                    .write_at(offset, &mut reader, StatusFlags::empty())?;
            if write_len != read_len {
                return_errno_with_message!(
                    Errno::EIO,
                    "the workdir temp accepted a short write during copy-up"
                );
            }
            offset += write_len;
        }
        // The invariant: every byte the lower declared must have been copied.
        if offset != size {
            return_errno_with_message!(
                Errno::EIO,
                "the copy-up copied fewer bytes than the lower file declares"
            );
        }
        Ok(())
    }

    /// Writes `self`'s three timestamps onto `temp_dentry`.
    pub(super) fn copy_timestamps_to(&self, temp_dentry: &Dentry) -> Result<()> {
        let source_inode = self.real_inode();
        let temp_inode = temp_dentry.inode();
        temp_inode.set_atime(temp_dentry, source_inode.atime());
        temp_inode.set_mtime(temp_dentry, source_inode.mtime());
        temp_inode.set_ctime(temp_dentry, source_inode.ctime());
        Ok(())
    }
}

/// The ordered real objects behind one logical overlay object, and which supplies its metadata.
#[derive(Debug)]
pub(super) struct RealObjectStack {
    pub(super) upper: Option<RealObject>,
    pub(super) lowers: Vec<RealObject>,
}

impl RealObjectStack {
    pub(super) fn new(upper: Option<RealObject>, lowers: Vec<RealObject>) -> Self {
        debug_assert!(upper.is_some() || !lowers.is_empty());
        Self { upper, lowers }
    }

    pub(super) fn upper_only(upper: RealObject) -> Self {
        Self {
            upper: Some(upper),
            lowers: Vec::new(),
        }
    }

    pub(super) fn lower_only(lower: RealObject) -> Self {
        Self {
            upper: None,
            lowers: vec![lower],
        }
    }

    pub(super) fn visible_source(&self) -> &RealObject {
        match &self.upper {
            Some(upper) => upper,
            None => &self.lowers[0],
        }
    }
}

/// Drives one real directory through `visitor` until it reports exhaustion.
pub(super) fn read_all_dirents(
    real_dir: &Arc<dyn Inode>,
    visitor: &mut dyn DirentVisitor,
) -> Result<()> {
    // A single `readdir_at` may stop short of the end, so the loop advances by what it reported.
    let mut offset = 0;
    loop {
        match real_dir.readdir_at(offset, visitor)? {
            0 => break,
            visited => offset += visited,
        }
    }
    Ok(())
}

pub(super) fn read_child_names(real_dir: &Arc<dyn Inode>) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    read_all_dirents(real_dir, &mut names)?;
    names.retain(|name| !path::is_dot_or_dotdot(name));
    Ok(names)
}
