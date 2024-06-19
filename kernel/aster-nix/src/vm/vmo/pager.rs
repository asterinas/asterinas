// SPDX-License-Identifier: MPL-2.0

use ostd::mm::Frame;

use crate::prelude::*;

/// Pagers provide frame to a VMO.
///
/// A `Pager` object can be attached to a VMO. Whenever the
/// VMO needs more frames (i.e., on commits), it will turn to the pager,
/// which should then provide frames whose data have been initialized properly.
/// Any time a frame is updated through the VMO, the VMO will
/// notify the attached pager that the frame has been updated.
/// Finally, when a frame is no longer needed (i.e., on decommits),
/// the frame pager will also be notified.
pub trait Pager: Send + Sync {
    /// Ask the pager to provide a frame at a specified index.
    ///
    /// After a page of a VMO is committed, the VMO shall not call this method
    /// again until the page is decommitted. But a robust implementation of
    /// `Pager` should not rely on this behavior for its correctness;
    /// instead, it should returns the _same_ frame.
    ///
    /// If a VMO page has been previously committed and decommited,
    /// and is to be committed again, then the pager is free to return
    /// whatever frame that may or may not be the same as the last time.
    ///
    /// It is up to the pager to decide the range of valid indices.
    fn commit_page(&self, idx: usize) -> Result<Frame>;

    /// Notify the pager that the frame at a specified index has been updated.
    ///
    /// Being aware of the updates allow the pager (e.g., an inode) to
    /// know which pages are dirty and only write back the _dirty_ pages back
    /// to disk.
    ///
    /// The VMO will not call this method for an uncommitted page.
    /// But a robust implementation of `Pager` should not make
    /// such an assumption for its correctness; instead, it should simply ignore the
    /// call or return an error.
    fn update_page(&self, idx: usize) -> Result<()>;

    /// Notify the pager that the frame at the specified index has been decommitted.
    ///
    /// Knowing that a frame is no longer needed, the pager (e.g., an inode)
    /// can free the frame after writing back its data to the disk.
    ///
    /// The VMO will not call this method for an uncommitted page.
    /// But a robust implementation of `Pager` should not make
    /// such an assumption for its correctness; instead, it should simply ignore the
    /// call or return an error.
    fn decommit_page(&self, idx: usize) -> Result<()>;

    /// Ask the pager to provide a frame at a specified index.
    /// Notify the pager that the frame will be fully overwritten soon, so pager can
    /// choose not to initialize it.
    fn commit_overwrite(&self, idx: usize) -> Result<Frame>;
}
