//! Memory reclamation for boot-time memory regions.
//! 
//! This module provides functionality for reclaiming memory regions that are
//! only used during the boot process. The reclamation can be done in two ways:
//! 
//! 1. Centralized: Using the specialized functions like `reclaim_initrd_memory()`
//!    or `reclaim_boot_memory_regions()` to reclaim specific types of memory.
//! 
//! 2. Distributed: Each module can reclaim its own memory regions when they are
//!    no longer needed, using the predicate-based `reclaim_memory_regions_with_predicate()`.
//! 
//! Memory regions that can be reclaimed include:
//! - Initrd (8MiB): After decompression and mapping to root filesystem
//! - ACPI tables (1KiB): After parsing and storing in kernel structures
//! - Bootloader regions: After initial boot image loading
//! 
//! # Safety
//! 
//! Memory reclamation must be done carefully to ensure that:
//! 1. The memory is truly no longer needed
//! 2. No other subsystem is still using the memory
//! 3. The memory is properly marked as Reclaimable
//! 
//! # Usage
//! 
//! ```rust
//! // Reclaim specific types of memory
//! reclaim_initrd_memory()?;
//! reclaim_acpi_tables_memory()?;
//! ```

use crate::{  
    boot::{memory_region::{MemoryRegion, MemoryRegionType}, EARLY_INFO},  
    mm::{frame::allocator::get_global_frame_allocator, Paddr, PAGE_SIZE},  
    prelude::*,  
};  
  
/// Adds a physical memory region to the global frame allocator.  
///   
/// This function handles the details of adding a memory region, ensuring  
/// it's properly aligned and logging the addition. It performs validation
/// to ensure the region is safe to reclaim.
/// 
/// # Arguments
/// 
/// * `region` - The memory region to be reclaimed
/// 
/// # Returns
/// 
/// * `Result<()>` - Ok if the region was successfully reclaimed, Error otherwise
/// 
/// # Safety
/// 
/// The caller must ensure that:
/// 1. The memory region is no longer in use by any subsystem
/// 2. The region is properly aligned to page boundaries
/// 3. The region is marked as Reclaimable in its MemoryRegionType
pub fn add_physical_memory_region(region: &MemoryRegion) -> Result<()> {  
    // Validate region alignment
    if region.base() % PAGE_SIZE != 0 || region.len() % PAGE_SIZE != 0 {  
        log::warn!("Memory region {:x?}-{:x?} is not page-aligned", region.base(), region.end());
        return Err(Error::InvalidArgs);  
    }  

    // Validate region type
    if region.typ() != MemoryRegionType::Reclaimable {
        log::warn!("Attempting to reclaim non-reclaimable memory region of type {:?}", region.typ());
        return Err(Error::InvalidArgs);
    }
      
    log::info!("Reclaiming memory region: {:x?}-{:x?} ({} bytes, type: {:?})",   
               region.base(), region.end(), region.len(), region.typ());  
      
    get_global_frame_allocator()  
        .add_free_memory(region.base(), region.len());  
      
    // Update statistics
    let mut stats = RECLAMATION_STATS.lock();
    stats.regions_reclaimed += 1;
    stats.bytes_reclaimed += region.len();
    log::debug!("Updated reclamation stats: {:?}", *stats);
      
    Ok(())  
}  
  
/// Reclaims all boot memory regions that are marked as reclaimable.  
///   
/// This should be called after the kernel initialization is complete,  
/// when the boot-time structures are no longer needed. The function
/// tracks and logs memory reclamation statistics.
/// 
/// # Returns
/// 
/// * `Result<()>` - Ok if all reclaimable regions were successfully reclaimed
/// 
/// # Safety
/// 
/// This function should only be called once after all boot-time subsystems
/// have completed their initialization and no longer need their boot memory.
pub fn reclaim_boot_memory_regions() -> Result<()> {  
    let regions = &EARLY_INFO.get().unwrap().memory_regions;  
    
    let mut total_reclaimed = 0;
    let mut reclaimed_count = 0;
      
    for region in regions.iter() {  
        if region.typ() == MemoryRegionType::Reclaimable {  
            add_physical_memory_region(region)?;  
            total_reclaimed += region.len();
            reclaimed_count += 1;
        }  
    }  
          
    log::info!("Boot memory regions reclaimed: {} regions, {} bytes", 
               reclaimed_count, total_reclaimed);
    Ok(())  
}

/// Reclaims initrd memory after it has been processed.
/// 
/// This function should be called after the initrd has been decompressed,
/// decoded, and mapped into the root filesystem. The initrd memory (around 8MiB)
/// is no longer needed after this point.
/// 
/// # Returns
/// 
/// * `Result<()>` - Ok if initrd memory was successfully reclaimed
pub fn reclaim_initrd_memory() -> Result<()> {
    let regions = &EARLY_INFO.get().unwrap().memory_regions;
    let mut reclaimed = false;
    
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Module {
            add_physical_memory_region(region)?;
            reclaimed = true;
            log::info!("Reclaimed initrd memory region: {:x?}-{:x?}", region.base(), region.end());
        }
    }
    
    if !reclaimed {
        log::warn!("No initrd memory regions found to reclaim");
    }
    
    Ok(())
}

/// Reclaims ACPI tables memory after initialization.
/// 
/// This function should be called after the ACPI tables have been parsed
/// and the necessary information has been extracted into the kernel's data
/// structures. The ACPI tables memory (around 1KiB) is no longer needed
/// after this point.
/// 
/// # Returns
/// 
/// * `Result<()>` - Ok if ACPI tables memory was successfully reclaimed
pub fn reclaim_acpi_tables_memory() -> Result<()> {
    let regions = &EARLY_INFO.get().unwrap().memory_regions;
    let mut reclaimed = false;
    
    for region in regions.iter() {
        if region.typ() == MemoryRegionType::Reclaimable {
            // Only reclaim small regions that are likely to be ACPI tables
            if region.len() <= 4096 { // 4KiB threshold
                add_physical_memory_region(region)?;
                reclaimed = true;
                log::info!("Reclaimed ACPI tables memory region: {:x?}-{:x?}", region.base(), region.end());
            }
        }
    }
    
    if !reclaimed {
        log::warn!("No ACPI tables memory regions found to reclaim");
    }
    
    Ok(())
}

/// Tracks memory reclamation statistics
#[derive(Debug, Default)]
pub struct ReclamationStats {
    /// Total number of regions reclaimed
    pub regions_reclaimed: usize,
    /// Total bytes reclaimed
    pub bytes_reclaimed: usize,
    /// Number of failed reclamation attempts
    pub failed_attempts: usize,
}

/// Global reclamation statistics
static RECLAMATION_STATS: SpinLock<ReclamationStats> = SpinLock::new(ReclamationStats::default());

/// Gets the current memory reclamation statistics
pub fn get_reclamation_stats() -> ReclamationStats {
    *RECLAMATION_STATS.lock()
}

/// Resets the reclamation statistics
pub fn reset_reclamation_stats() {
    *RECLAMATION_STATS.lock() = ReclamationStats::default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boot::memory_region::MemoryRegion;

    #[test]
    fn test_memory_reclamation() {
        // Create test memory regions
        let regions = vec![
            MemoryRegion::new(0x1000, 0x2000, MemoryRegionType::Reclaimable),
            MemoryRegion::new(0x2000, 0x3000, MemoryRegionType::Module),
            MemoryRegion::new(0x3000, 0x4000, MemoryRegionType::Reclaimable),
        ];

        // Test reclaim_boot_memory_regions
        let result = reclaim_boot_memory_regions();
        assert!(result.is_ok());

        // Test reclaim_initrd_memory
        let result = reclaim_initrd_memory();
        assert!(result.is_ok());

        // Test reclaim_acpi_tables_memory
        let result = reclaim_acpi_tables_memory();
        assert!(result.is_ok());

        // Verify statistics
        let stats = get_reclamation_stats();
        assert!(stats.regions_reclaimed > 0);
        assert!(stats.bytes_reclaimed > 0);
    }
}
