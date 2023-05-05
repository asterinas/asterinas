use acpi::PciConfigRegions;

pub fn start_address() -> usize {
    let start = PciConfigRegions::new(
        &*crate::arch::x86::kernel::acpi::ACPI_TABLES
            .get()
            .unwrap()
            .lock(),
    )
    .unwrap();

    // all zero to get the start address
    start.physical_address(0, 0, 0, 0).unwrap() as usize
}

pub fn end_address() -> usize {
    // 4G-20M
    0xFEC0_0000
}
