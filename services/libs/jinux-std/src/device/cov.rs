//! Coverage device in /dev/cov, read it to get kernel coverage data
//!

use super::*;
use crate::prelude::*;

pub struct Cov;

impl Device for Cov {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(0xdead, 0xbeef)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let cov = jinux_frame::get_llvm_coverage_raw();
        // Since device have no state, we cannot store where we were read at.
        // Just ask the user to prepare a buffer large enough (typically > 7M).
        assert!(buf.len() >= cov.len());
        for (src, dst) in cov.iter().zip(buf) {
            *dst = *src;
        }
        Ok(cov.len())
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
}
