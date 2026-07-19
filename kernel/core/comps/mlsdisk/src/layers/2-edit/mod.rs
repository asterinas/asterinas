// SPDX-License-Identifier: MPL-2.0

//! The layer of edit journal.

mod edits;
mod journal;

pub use self::{
    edits::{Edit, EditGroup},
    journal::{
        CompactPolicy, DefaultCompactPolicy, EditJournal, EditJournalMeta, NeverCompactPolicy,
    },
};
