// SPDX-License-Identifier: MPL-2.0

use aster_frame::{
    config::PAGE_SIZE,
    vm::{Daddr, DmaCoherent, DmaStream, DmaStreamSlice, HasDaddr},
};

#[allow(clippy::len_without_is_empty)]
pub trait DmaBuf {
    fn addr(&self) -> Daddr;
    fn len(&self) -> usize;
}

impl DmaBuf for DmaStream {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for DmaStreamSlice {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for DmaCoherent {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.nframes() * PAGE_SIZE
    }
}

impl DmaBuf for (Daddr, usize) {
    fn addr(&self) -> Daddr {
        self.0
    }

    fn len(&self) -> usize {
        self.1
    }
}
