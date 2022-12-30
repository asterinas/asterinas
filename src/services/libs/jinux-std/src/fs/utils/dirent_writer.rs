use super::InodeType;
use crate::prelude::*;

/// DirentWriterContext is a wrapper of DirentWriter with directory position
/// After a successful write, the position increases correspondingly
pub struct DirentWriterContext<'a> {
    pos: usize,
    writer: &'a mut dyn DirentWriter,
}

impl<'a> DirentWriterContext<'a> {
    pub fn new(pos: usize, writer: &'a mut dyn DirentWriter) -> Self {
        Self { pos, writer }
    }

    pub fn write_entry(&mut self, name: &str, ino: u64, type_: InodeType) -> Result<usize> {
        let written_len = self.writer.write_entry(name, ino, type_)?;
        self.pos += 1;
        Ok(written_len)
    }

    pub fn pos(&self) -> usize {
        self.pos
    }
}

/// DirentWriter is used to write directory entry,
/// the object which implements it can decide how to format the data
pub trait DirentWriter: Sync + Send {
    fn write_entry(&mut self, name: &str, ino: u64, type_: InodeType) -> Result<usize>;
}
