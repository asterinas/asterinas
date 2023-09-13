use crate::bid::{BlockId, BLOCK_SIZE};
use crate::prelude::*;

#[derive(Debug)]
pub struct Bio<'a> {
    bio_bufs: Vec<BioBufDescriptor<'a>>,
    idx: usize,
    type_: BioType,
}

impl<'a> Bio<'a> {
    pub fn new(bio_bufs: Vec<BioBufDescriptor<'a>>, type_: BioType) -> Self {
        Self {
            bio_bufs,
            idx: 0,
            type_,
        }
    }

    pub fn from_bytes_at(buf: &'a [u8], mut offset: usize) -> Self {
        let mut bio_bufs = Vec::new();
        let mut remaining_slice = buf;

        loop {
            let offset_in_block = BlockId::offset_in_block(offset);
            let bio_buf_len = (remaining_slice.len()).min(BLOCK_SIZE - offset_in_block);
            if bio_buf_len == 0 {
                break;
            }

            let (bio_buf_slice, new_remaining_slice) = remaining_slice.split_at(bio_buf_len);
            let bio_buf_des = {
                let bio_buf = BioBuf::from_slice(bio_buf_slice);
                let bid = BlockId::from_offset(offset);
                BioBufDescriptor::new(bio_buf, bid, offset_in_block).unwrap()
            };
            bio_bufs.push(bio_buf_des);

            offset += bio_buf_len;
            remaining_slice = new_remaining_slice;
        }

        Self::new(bio_bufs, BioType::Write)
    }

    pub fn from_bytes_mut_at(buf: &'a mut [u8], mut offset: usize) -> Self {
        let mut bio_bufs = Vec::new();
        let mut remaining_slice = buf;

        loop {
            let offset_in_block = BlockId::offset_in_block(offset);
            let bio_buf_len = (remaining_slice.len()).min(BLOCK_SIZE - offset_in_block);
            if bio_buf_len == 0 {
                break;
            }

            let (bio_buf_slice, new_remaining_slice) = remaining_slice.split_at_mut(bio_buf_len);
            let bio_buf_des = {
                let bio_buf = BioBuf::from_slice_mut(bio_buf_slice);
                let bid = BlockId::from_offset(offset);
                BioBufDescriptor::new(bio_buf, bid, offset_in_block).unwrap()
            };
            bio_bufs.push(bio_buf_des);

            offset += bio_buf_len;
            remaining_slice = new_remaining_slice;
        }

        Self::new(bio_bufs, BioType::Read)
    }

    pub fn bio_bufs(&self) -> &Vec<BioBufDescriptor<'a>> {
        &self.bio_bufs
    }

    pub fn bio_bufs_mut(&mut self) -> &mut Vec<BioBufDescriptor<'a>> {
        &mut self.bio_bufs
    }

    pub fn bio_type(&self) -> BioType {
        self.type_
    }

    pub fn idx(&self) -> usize {
        self.idx
    }

    pub fn set_idx(&mut self, new_idx: usize) {
        debug_assert!(new_idx <= self.bio_bufs.len());
        self.idx = new_idx;
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BioType {
    Read = 0,
    Write = 1,
}

#[derive(Debug)]
pub struct BioBufDescriptor<'a> {
    /// The buffer.
    buf: BioBuf<'a>,
    /// The block id of the block where the buffer resides.
    bid: BlockId,
    /// The byte offset within the block where the buffer resides.
    offset: usize,
}

impl<'a> BioBufDescriptor<'a> {
    pub fn new(buf: BioBuf<'a>, bid: BlockId, offset: usize) -> Result<Self> {
        if offset + buf.len() > BLOCK_SIZE {
            return Err(Error::InvalidArgs);
        }

        Ok(Self { buf, bid, offset })
    }

    pub fn buf_mut(&mut self) -> &mut BioBuf<'a> {
        &mut self.buf
    }

    pub fn buf(&self) -> &BioBuf<'a> {
        &self.buf
    }

    pub fn bid(&self) -> BlockId {
        self.bid
    }

    pub fn set_bid(&mut self, bid: BlockId) {
        self.bid = bid;
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

#[derive(Debug)]
pub enum BioBuf<'a> {
    Borrow(&'a [u8]),
    BorrowMut(&'a mut [u8]),
}

impl<'a> BioBuf<'a> {
    pub fn from_slice(slice: &'a [u8]) -> Self {
        debug_assert!(slice.len() <= BLOCK_SIZE);
        Self::Borrow(slice)
    }

    pub fn from_slice_mut(slice_mut: &'a mut [u8]) -> Self {
        debug_assert!(slice_mut.len() <= BLOCK_SIZE);
        Self::BorrowMut(slice_mut)
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Borrow(slice) => slice,
            Self::BorrowMut(slice_mut) => slice_mut,
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::BorrowMut(slice_mut) => slice_mut,
            Self::Borrow(_) => panic!(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Borrow(slice) => slice.len(),
            Self::BorrowMut(slice_mut) => slice_mut.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a> MemStorage for BioBuf<'a> {
    fn mem_areas(&self, is_writable: bool) -> Result<MemStorageIterator> {
        let mem_area = if is_writable {
            if let BioBuf::Borrow(_) = self {
                return Err(Error::AccessDenied);
            }
            // Safty: BorrowMut slice occupy it exclusively
            unsafe { MemArea::from_raw_parts_mut(self.as_slice().as_ptr() as *mut u8, self.len()) }
        } else {
            MemArea::from_slice(self.as_slice())
        };

        Ok(MemStorageIterator::from_vec(vec![mem_area]))
    }

    fn total_len(&self) -> usize {
        self.len()
    }
}
