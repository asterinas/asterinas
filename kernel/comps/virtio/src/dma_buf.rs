// SPDX-License-Identifier: MPL-2.0

use aster_frame::{
    config::PAGE_SIZE,
    vm::{Daddr, DmaCoherent, DmaStream, HasDaddr},
};
use aster_network::{DmaBlock, RxBuffer, TxBuffer};

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

impl DmaBuf for DmaBlock {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.size()
    }
}

impl DmaBuf for TxBuffer {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.nbytes()
    }
}

impl DmaBuf for RxBuffer {
    fn addr(&self) -> Daddr {
        self.daddr()
    }

    fn len(&self) -> usize {
        self.buf_len()
    }
}
