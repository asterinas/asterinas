// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use serde::{ser::SerializeSeq, Deserialize, Serialize};

use crate::prelude::*;

/// An edit of `Edit<S>` is an incremental change to a state of `S`.
pub trait Edit<S>: Serialize + for<'de> Deserialize<'de> {
    /// Apply this edit to a state.
    fn apply_to(&self, state: &mut S);
}

/// A group of edits to a state.
pub struct EditGroup<E: Edit<S>, S> {
    edits: Vec<E>,
    _s: PhantomData<S>,
}

impl<E: Edit<S>, S> EditGroup<E, S> {
    /// Creates an empty edit group.
    pub fn new() -> Self {
        Self {
            edits: Vec::new(),
            _s: PhantomData,
        }
    }

    /// Adds an edit to the group.
    pub fn push(&mut self, edit: E) {
        self.edits.push(edit);
    }

    /// Returns an iterator to the contained edits.
    pub fn iter(&self) -> impl Iterator<Item = &E> {
        self.edits.iter()
    }

    /// Clears the edit group by removing all contained edits.
    pub fn clear(&mut self) {
        self.edits.clear()
    }

    /// Returns whether the edit group contains no edits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the length of the edit group.
    pub fn len(&self) -> usize {
        self.edits.len()
    }
}

impl<E: Edit<S>, S> Edit<S> for EditGroup<E, S> {
    fn apply_to(&self, state: &mut S) {
        for edit in &self.edits {
            edit.apply_to(state);
        }
    }
}

impl<E: Edit<S>, S> Serialize for EditGroup<E, S> {
    fn serialize<Se>(&self, serializer: Se) -> core::result::Result<Se::Ok, Se::Error>
    where
        Se: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for edit in &self.edits {
            seq.serialize_element(edit)?
        }
        seq.end()
    }
}

impl<'de, E: Edit<S>, S> Deserialize<'de> for EditGroup<E, S> {
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EditsVisitor<E: Edit<S>, S> {
            _p: PhantomData<(E, S)>,
        }

        impl<'a, E: Edit<S>, S> serde::de::Visitor<'a> for EditsVisitor<E, S> {
            type Value = EditGroup<E, S>;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                formatter.write_str("an edit group")
            }

            fn visit_seq<A>(self, mut seq: A) -> core::result::Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'a>,
            {
                let mut edits = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(e) = seq.next_element()? {
                    edits.push(e);
                }
                Ok(EditGroup {
                    edits,
                    _s: PhantomData,
                })
            }
        }

        deserializer.deserialize_seq(EditsVisitor { _p: PhantomData })
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct XEdit {
        x: i32,
    }

    struct XState {
        sum: i32,
    }

    impl Edit<XState> for XEdit {
        fn apply_to(&self, state: &mut XState) {
            (*state).sum += self.x;
        }
    }

    #[test]
    fn serde_edit() {
        let mut group = EditGroup::<XEdit, XState>::new();
        let mut sum = 0;
        for x in 0..10 {
            sum += x;
            let edit = XEdit { x };
            group.push(edit);
        }
        let mut state = XState { sum: 0 };
        group.apply_to(&mut state);
        assert_eq!(state.sum, sum);

        let mut buf = [0u8; 64];
        let ser = postcard::to_slice(&group, buf.as_mut_slice()).unwrap();
        println!("serialize len: {} data: {:?}", ser.len(), ser);
        let de: EditGroup<XEdit, XState> = postcard::from_bytes(buf.as_slice()).unwrap();
        println!("deserialize edits: {:?}", de.edits);
        assert_eq!(de.len(), group.len());
        assert_eq!(de.edits.as_slice(), group.edits.as_slice());
    }
}
