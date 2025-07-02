// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::ptr;

use spin::Once;

use crate::mm::kspace::paddr_to_vaddr;

/// Coverage collector that manages code coverage data and shared memory operations
pub struct CoverageCollector {
    shared_mem_paddr: u64,
    shared_mem_size: usize,
    coverage_data: Vec<u8>,
}

impl CoverageCollector {
    /// Create a new coverage collector with specified physical address and size
    pub fn new(paddr: u64, size: usize) -> Self {
        Self {
            shared_mem_paddr: paddr,
            shared_mem_size: size,
            coverage_data: Vec::new(),
        }
    }

    /// Collect current coverage data using minicov
    pub fn collect_coverage_data(&mut self) {
        self.coverage_data.clear();
        unsafe {
            if let Err(e) = minicov::capture_coverage(&mut self.coverage_data) {
                log::warn!("Failed to capture coverage: {:?}", e);
            }
        }
    }

    /// Dump collected coverage data to the shared memory region
    pub fn dump_to_shared_memory(&self) -> Result<(), &'static str> {
        if self.coverage_data.len() > self.shared_mem_size {
            return Err("Coverage data too large for shared memory");
        }

        // Convert physical address to virtual address using kernel mapping
        let vaddr = paddr_to_vaddr(self.shared_mem_paddr as usize);

        // Write coverage data size as header (first 8 bytes)
        let data_size = self.coverage_data.len() as u64;
        unsafe {
            ptr::write_volatile(vaddr as *mut u64, data_size);
        }

        // Write actual coverage data
        if !self.coverage_data.is_empty() {
            unsafe {
                ptr::copy_nonoverlapping(
                    self.coverage_data.as_ptr(),
                    (vaddr + 8) as *mut u8,
                    self.coverage_data.len(),
                );
            }
        }

        log::debug!(
            "Dumped {} bytes of coverage data to shared memory at paddr 0x{:x}",
            self.coverage_data.len(),
            self.shared_mem_paddr
        );
        Ok(())
    }

    /// Get the size of collected coverage data
    pub fn coverage_data_size(&self) -> usize {
        self.coverage_data.len()
    }
}

/// Global coverage collector instance
static COVERAGE_COLLECTOR: Once<spin::Mutex<Option<CoverageCollector>>> = Once::new();

/// Initialize the coverage collector with shared memory parameters
pub fn init_coverage_collector(paddr: u64, size: usize) {
    COVERAGE_COLLECTOR.call_once(|| {
        let collector = CoverageCollector::new(paddr, size);
        log::debug!(
            "Initialized coverage collector with shared memory at paddr 0x{:x}, size 0x{:x} bytes",
            paddr,
            size
        );
        spin::Mutex::new(Some(collector))
    });
}

/// Collect and dump coverage data to shared memory
/// This should be called before VM exit
pub fn dump_coverage_before_exit() {
    if let Some(collector_mutex) = COVERAGE_COLLECTOR.get() {
        let mut collector_guard = collector_mutex.lock();
        if let Some(collector) = collector_guard.as_mut() {
            collector.collect_coverage_data();
            if let Err(e) = collector.dump_to_shared_memory() {
                log::warn!("Failed to dump coverage data: {}", e);
            }
        }
    }
}

/// Parse kernel command line for coverage parameters
pub fn parse_coverage_cmdline(cmdline: &str) -> Option<(u64, usize)> {
    for arg in cmdline.split_whitespace() {
        if let Some(paddr_str) = arg.strip_prefix("coverage_paddr=") {
            if let Ok(paddr) = parse_hex_or_dec(paddr_str) {
                // Default size is 16MB if not specified
                let size = parse_coverage_size(cmdline).unwrap_or(16 * 1024 * 1024);
                return Some((paddr, size));
            }
        }
    }
    None
}

/// Parse coverage memory size from command line
fn parse_coverage_size(cmdline: &str) -> Option<usize> {
    for arg in cmdline.split_whitespace() {
        if let Some(size_str) = arg.strip_prefix("coverage_size=") {
            if let Ok(size) = parse_hex_or_dec(size_str) {
                return Some(size as usize);
            }
        }
    }
    None
}

/// Parse hex (0x prefix) or decimal number
fn parse_hex_or_dec(s: &str) -> Result<u64, core::num::ParseIntError> {
    if let Some(hex_str) = s.strip_prefix("0x") {
        u64::from_str_radix(hex_str, 16)
    } else {
        s.parse::<u64>()
    }
}
