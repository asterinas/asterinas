// SPDX-License-Identifier: MPL-2.0

extern crate alloc;
use alloc::{boxed::Box, string::ToString, sync::Arc, vec::Vec};
use core::{hint::spin_loop, mem::offset_of};

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use ostd::{
    arch::{
        boot::DEVICE_TREE,
        irq::{InterruptSourceInFdt, MappedIrqLine, IRQ_CHIP},
    },
    io::IoMem,
    irq::IrqLine,
    mm::{VmIoOnce, VmReader},
    sync::{LocalIrqDisabled, Rcu, SpinLock},
};
use spin::Once;

use crate::Uart;

pub fn init() {
    let fdt = DEVICE_TREE.get().unwrap();
    let uart_nodes = fdt.all_nodes().filter(|n| {
        n.compatible()
            .is_some_and(|c| c.all().any(|s| s == SifiveUart::FDT_COMPATIBLE))
    });

    let stdout = fdt.chosen().stdout().map(|node| node.name);

    let mut mapped_irq_lines: Vec<MappedIrqLine> = Vec::new();
    uart_nodes.for_each(|uart_node| {
        let reg = uart_node.reg().unwrap().next().unwrap();
        let io_mem = IoMem::acquire(
            reg.starting_address as usize..reg.starting_address as usize + reg.size.unwrap(),
        )
        .unwrap();

        let interrupt_source_in_fdt = InterruptSourceInFdt {
            interrupt: uart_node.interrupts().unwrap().next().unwrap() as u32,
            interrupt_parent: uart_node
                .property("interrupt-parent")
                .and_then(|prop| prop.as_usize())
                .unwrap() as u32,
        };
        let mut mapped_irq_line = IrqLine::alloc()
            .and_then(|irq_line| {
                IRQ_CHIP
                    .get()
                    .unwrap()
                    .map_fdt_pin_to(interrupt_source_in_fdt, irq_line)
            })
            .unwrap();

        let uart = Arc::new(SifiveUart {
            io_mem,
            callbacks: Rcu::new(Box::new(Vec::new())),
            tx_lock: SpinLock::new(()),
        });
        uart.init(SifiveUart::CLOCK_HZ);

        if let Some(stdout_path) = stdout
            && stdout_path == uart_node.name
        {
            aster_console::register_device(
                format_args!("SIFIVE_UART_{}_CONSOLE", mapped_irq_lines.len()).to_string(),
                uart.clone(),
            );
        }

        mapped_irq_line.on_active(move |_trapframe| {
            uart.handle_rx_irq();
        });
        mapped_irq_lines.push(mapped_irq_line);
    });

    SIFIVE_UART_MAPPED_IRQ_LINES.call_once(|| mapped_irq_lines);
}

static SIFIVE_UART_MAPPED_IRQ_LINES: Once<Vec<MappedIrqLine>> = Once::new();

pub struct SifiveUart {
    io_mem: IoMem,
    #[expect(clippy::box_collection)]
    callbacks: Rcu<Box<Vec<&'static ConsoleCallback>>>,
    // We need to lock transmit operations to ensure that multiple threads
    // do not interleave their writes.
    tx_lock: SpinLock<(), LocalIrqDisabled>,
}

impl Uart for SifiveUart {
    fn init(&self, clock_hz: u32) {
        let div = ((clock_hz as u64 + (Self::TARGET_BAUD as u64 / 2)) / (Self::TARGET_BAUD as u64)
            - 1) as u32;
        self.io_mem
            .write_once(offset_of!(Registers, div), &div)
            .unwrap();
        self.io_mem
            .write_once(offset_of!(Registers, txctrl), &Self::TXCTRL_TXEN)
            .unwrap();
        self.io_mem
            .write_once(
                offset_of!(Registers, rxctrl),
                &(Self::RXCTRL_RXEN | (0 << Self::RXCTRL_RXCNT_SHIFT)),
            )
            .unwrap();
        self.io_mem
            .write_once(offset_of!(Registers, ie), &Self::IE_RXWM)
            .unwrap();
    }

    fn transmit(&self, byte: u8) -> ostd::Result<()> {
        let offset = offset_of!(Registers, txdata);
        let txdata = self.io_mem.read_once::<u32>(offset).unwrap();
        if txdata & Self::TXDATA_FULL != 0 {
            Err(ostd::Error::NotEnoughResources)
        } else {
            self.io_mem
                .write_once(offset, &(byte as u32 & Self::TXDATA_DATA_MASK))
                .unwrap();
            Ok(())
        }
    }

    fn receive(&self) -> Option<u8> {
        let offset = offset_of!(Registers, rxdata);
        let rxdata = self.io_mem.read_once::<u32>(offset).unwrap();
        if rxdata & Self::RXDATA_EMPTY != 0 {
            None
        } else {
            Some((rxdata & Self::RXDATA_DATA_MASK) as u8)
        }
    }
}

impl AnyConsoleDevice for SifiveUart {
    fn send(&self, bytes: &[u8]) {
        let _tx_lock = self.tx_lock.lock();
        for &byte in bytes {
            self.transmit_blocking(byte);
        }
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        loop {
            let callbacks = self.callbacks.read();
            let mut callbacks_cloned = callbacks.get().clone();
            callbacks_cloned.push(callback);
            if callbacks.compare_exchange(callbacks_cloned).is_ok() {
                break;
            }
            // Contention on pushing, retry.
            core::hint::spin_loop();
        }
    }
}

impl SifiveUart {
    fn transmit_blocking(&self, byte: u8) {
        while self.transmit(byte).is_err() {
            spin_loop();
        }
    }

    fn handle_rx_irq(&self) {
        let mut buffer = [0u8; 128];
        loop {
            let mut count = 0;
            while let Some(byte) = self.receive() {
                buffer[count] = byte;
                count += 1;
                if count >= buffer.len() {
                    break;
                }
            }

            if count == 0 {
                break;
            }

            let callbacks = self.callbacks.read();
            for callback in callbacks.get().iter() {
                let reader = VmReader::from(&buffer[..count]);
                callback(reader);
            }
            drop(callbacks);
        }
    }
}

impl SifiveUart {
    const FDT_COMPATIBLE: &'static str = "sifive,uart0";

    const TARGET_BAUD: u32 = 115200;
    // FIXME: We should query the clock frequency from the device tree. Here we
    // hardcode it to 500MHz for SiFive Hifive Unleashed board.
    const CLOCK_HZ: u32 = 500_000_000;

    const TXDATA_FULL: u32 = 0b1 << 31;
    const TXDATA_DATA_MASK: u32 = 0xff;
    const RXDATA_EMPTY: u32 = 0b1 << 31;
    const RXDATA_DATA_MASK: u32 = 0xff;
    const TXCTRL_TXEN: u32 = 0b1;
    const RXCTRL_RXEN: u32 = 0b1;
    const RXCTRL_RXCNT_SHIFT: u32 = 16;
    const IE_RXWM: u32 = 0b1 << 1;
}

impl core::fmt::Debug for SifiveUart {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SifiveUart").finish_non_exhaustive()
    }
}

#[repr(C)]
struct Registers {
    txdata: u32,
    rxdata: u32,
    txctrl: u32,
    rxctrl: u32,
    ie: u32,
    _ip: u32,
    div: u32,
}
