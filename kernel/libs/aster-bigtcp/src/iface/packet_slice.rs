// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use ostd::mm::{Infallible, VmReader, VmWriter};
use smoltcp::storage::SliceLike;

#[derive(Clone)]
pub(crate) struct PacketSlice<'a>(VmReader<'a, Infallible>);

impl<'a> From<&'a [u8]> for PacketSlice<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self(VmReader::from(value))
    }
}

impl<'a> From<VmReader<'a, Infallible>> for PacketSlice<'a> {
    fn from(value: VmReader<'a, Infallible>) -> Self {
        Self(value)
    }
}

impl SliceLike for PacketSlice<'_> {
    type Item = u8;

    fn len(&self) -> usize {
        self.0.remain()
    }

    fn index(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end);
        assert!(range.end <= self.0.remain());

        let mut reader = self.0.clone();
        reader.skip(range.start).limit(range.end - range.start);
        Self(reader)
    }

    fn copy_to_slice(&self, output: &mut [u8]) {
        assert_eq!(self.0.remain(), output.len());

        let mut reader = self.0.clone();
        let read_len = reader.read(&mut VmWriter::from(&mut *output));
        debug_assert_eq!(read_len, output.len());
    }
}
