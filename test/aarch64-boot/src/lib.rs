// SPDX-License-Identifier: MPL-2.0
#![no_std]
#![deny(unsafe_code)]

use ostd::prelude::*;

#[ostd::main]
fn kernel_main() {
    println!("========================================");
    println!(" Hello from Asterinas on AArch64!");
    println!("========================================");
    let boot_info = ostd::boot::boot_info();
    println!("[aarch64-boot] bootloader: {}", boot_info.bootloader_name);
    println!("[aarch64-boot] cmdline: {}", boot_info.kernel_cmdline);
    println!(
        "[aarch64-boot] memory regions: {}",
        boot_info.memory_regions.len()
    );
    for region in boot_info.memory_regions.iter() {
        println!(
            "[aarch64-boot]   {:#x}..{:#x} {:?}",
            region.base(),
            region.end(),
            region.typ()
        );
    }
    println!("[aarch64-boot] TSC freq: {} Hz", ostd::arch::tsc_freq());
    println!("[aarch64-boot] Boot reached kernel_main successfully.");

    // Demonstrate that GIC + generic-timer interrupts are firing: spin until the
    // jiffies counter (driven by the timer IRQ) advances several times.
    println!("[aarch64-boot] Waiting for timer interrupts...");
    let start = ostd::timer::Jiffies::elapsed().as_u64();
    let mut last = start;
    let mut ticks_seen = 0;
    while ticks_seen < 5 {
        let now = ostd::timer::Jiffies::elapsed().as_u64();
        if now != last {
            ticks_seen += 1;
            last = now;
            println!("[aarch64-boot]   timer tick! jiffies = {}", now);
        }
        core::hint::spin_loop();
    }
    println!(
        "[aarch64-boot] SUCCESS: {} timer interrupts observed (jiffies {} -> {}).",
        ticks_seen, start, last
    );
    println!("[aarch64-boot] Full aarch64 boot with working interrupts. Halting.");
}
