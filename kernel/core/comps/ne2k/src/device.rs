// SPDX-License-Identifier: GPL-1.0+
// Derived from Linux ne2k-pci.c and lib8390.c; see lib.rs for notices.

use alloc::{collections::VecDeque, string::ToString, sync::Arc, vec::Vec};
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_bigtcp::{
    device::{AnyNetworkDevice, DeviceCapabilities, EthernetAddress, Medium, NetError},
    packet::{FreshTxPacket, LinkLayer, RxPacket, TxPacket},
};
use aster_pci::{
    PCI_BUS, PciDeviceId,
    bus::{PciDevice, PciDriver},
    cfg_space::{Bar, Command, PciCommonCfgOffset},
    common_device::PciCommonDevice,
};
use aster_softirq::{BottomHalfDisabled, Taskless};
use ostd::{
    Error, Result,
    arch::{
        device::io_port::ReadWriteAccess,
        irq::{IRQ_CHIP, MappedIrqLine},
    },
    bus::BusProbeError,
    io::IoPort,
    irq::IrqLine,
    mm::{VmReader, VmWriter},
    sync::{LocalIrqDisabled, SpinLock},
};

use super::registers::*;

struct Ports {
    registers: Vec<Option<IoPort<u8, ReadWriteAccess>>>,
    data: IoPort<u16, ReadWriteAccess>,
}

impl Ports {
    fn acquire(base: u16) -> Result<Self> {
        let mut registers = Vec::with_capacity(32);
        for offset in 0..32u16 {
            if offset == 0x10 || offset == 0x11 {
                registers.push(None);
            } else {
                registers.push(Some(IoPort::acquire(base + offset)?));
            }
        }
        let data = IoPort::acquire(base + 0x10)?;
        Ok(Self { registers, data })
    }

    fn read(&self, offset: usize) -> u8 {
        self.registers[offset].as_ref().unwrap().read()
    }
    fn write(&self, offset: usize, value: u8) {
        self.registers[offset].as_ref().unwrap().write(value);
    }
}

struct State {
    ports: Ports,
    mac: [u8; 6],
    running: bool,
    generation: usize,
    rx: VecDeque<RxPacket<LinkLayer>>,
    slots: [Option<usize>; 2],
    active: Option<usize>,
    tx_started: u64,
}

impl State {
    fn read_reg(&self, register: usize) -> u8 {
        self.ports.read(register)
    }
    fn write_reg(&self, register: usize, value: u8) {
        self.ports.write(register, value);
    }

    fn reset(&self) -> Result<()> {
        self.write_reg(CR, CMD_STOP);
        self.write_reg(IMR, 0);
        let value = self.read_reg(RESET);
        self.write_reg(RESET, value);
        for _ in 0..POLL_LIMIT {
            if self.read_reg(ISR) & RESET_DONE != 0 {
                self.write_reg(ISR, ACK_ALL);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::IoError)
    }

    fn read_prom(&self) -> [u8; 6] {
        self.write_reg(CR, CMD_STOP);
        self.write_reg(DCR, DCR_WORD);
        self.write_reg(RBCR0, 0);
        self.write_reg(RBCR1, 0);
        self.write_reg(IMR, 0);
        self.write_reg(ISR, ACK_ALL);
        self.write_reg(RCR, RCR_MONITOR);
        self.write_reg(TCR, TCR_LOOPBACK);
        self.write_reg(RBCR0, 32);
        self.write_reg(RBCR1, 0);
        self.write_reg(RSAR0, 0);
        self.write_reg(RSAR1, 0);
        self.write_reg(CR, CMD_READ);
        let mut prom = [0; 32];
        // The 16-bit RTL8029 PROM duplicates each MAC byte in both lanes.
        for pair in prom.as_chunks_mut::<2>().0 {
            pair.copy_from_slice(&self.ports.data.read().to_le_bytes());
        }
        self.write_reg(ISR, RDC);
        let mut mac = [0; 6];
        for (index, byte) in mac.iter_mut().enumerate() {
            *byte = prom[index * 2];
        }
        mac
    }

    fn initialize(&mut self, mode: InitializeMode) {
        self.running = false;
        self.generation = self.generation.wrapping_add(1);
        self.write_reg(CR, CMD_STOP);
        self.write_reg(IMR, 0);
        self.write_reg(DCR, DCR_WORD);
        self.write_reg(RBCR0, 0);
        self.write_reg(RBCR1, 0);
        self.write_reg(RCR, RCR_MONITOR);
        self.write_reg(TCR, TCR_LOOPBACK);
        self.write_reg(TPSR, TX_START);
        self.write_reg(PSTART, RX_START);
        self.write_reg(PSTOP, RX_END);
        self.write_reg(BNRY, RX_END - 1);
        self.write_reg(ISR, ACK_ALL);
        self.write_reg(CR, CMD_PAGE1_STOP);
        for i in 0..6 {
            self.write_reg(PAR0 + i, self.mac[i]);
        }
        self.write_reg(CURR, RX_START);
        for i in MAR0..MAR0 + 8 {
            self.write_reg(i, 0);
        }
        self.write_reg(CR, CMD_STOP);
        self.rx.clear();
        self.slots = [None, None];
        self.active = None;
        if matches!(mode, InitializeMode::Running) {
            self.write_reg(ISR, ACK_ALL);
            self.write_reg(CR, CMD_START);
            self.write_reg(TCR, 0);
            self.write_reg(RCR, RCR_BROADCAST);
            self.write_reg(IMR, IRQ_MASK);
            self.running = true;
        }
    }

    fn stop(&mut self) {
        self.initialize(InitializeMode::Stopped);
    }

    fn remote_read(&self, addr: u16, out: &mut [u8]) {
        if out.is_empty() {
            return;
        }
        let count = out.len().next_multiple_of(2);
        self.write_reg(CR, CMD_START);
        self.write_reg(RBCR0, count as u8);
        self.write_reg(RBCR1, (count >> 8) as u8);
        self.write_reg(RSAR0, addr as u8);
        self.write_reg(RSAR1, (addr >> 8) as u8);
        self.write_reg(CR, CMD_READ);
        for pair in out.chunks_mut(2) {
            let word = self.ports.data.read().to_le_bytes();
            pair[0] = word[0];
            if pair.len() == 2 {
                pair[1] = word[1];
            }
        }
        self.write_reg(ISR, RDC);
    }

    fn remote_write(&self, page: u8, bytes: &[u8]) -> Result<()> {
        let count = bytes.len() + (bytes.len() & 1);
        self.write_reg(CR, CMD_START);
        self.write_reg(ISR, RDC);
        self.write_reg(RBCR0, count as u8);
        self.write_reg(RBCR1, (count >> 8) as u8);
        self.write_reg(RSAR0, 0);
        self.write_reg(RSAR1, page);
        self.write_reg(CR, CMD_WRITE);
        for pair in bytes.chunks(2) {
            self.ports
                .data
                .write(u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
        }
        for _ in 0..POLL_LIMIT {
            if self.read_reg(ISR) & RDC != 0 {
                self.write_reg(ISR, RDC);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::IoError)
    }

    fn trigger(&mut self, slot: usize) {
        let length = self.slots[slot].unwrap();
        self.write_reg(CR, CMD_START);
        self.write_reg(TBCR0, length as u8);
        self.write_reg(TBCR1, (length >> 8) as u8);
        self.write_reg(TPSR, TX_START + slot as u8 * (TX_PAGES / 2));
        self.write_reg(CR, CMD_TRANSMIT);
        self.active = Some(slot);
        self.tx_started = ostd::arch::read_tsc();
    }

    fn tx_complete(&mut self) {
        if let Some(slot) = self.active.take() {
            self.slots[slot] = None;
        }
        if let Some(slot) = self.slots.iter().position(Option::is_some) {
            self.trigger(slot);
        }
    }

    fn reap_tx(&mut self) {
        if !self.running {
            return;
        }
        self.write_reg(CR, CMD_START);
        let status = self.read_reg(ISR) & TX_IRQ;
        if status != 0 {
            self.write_reg(ISR, status);
            self.tx_complete();
        }
    }

    fn read_ring(&self, addr: u16, out: &mut [u8]) {
        let end = (RX_END as usize) << 8;
        let first = out.len().min(end.saturating_sub(addr as usize));
        self.remote_read(addr, &mut out[..first]);
        if first < out.len() {
            self.remote_read((RX_START as u16) << 8, &mut out[first..]);
        }
    }

    fn drain_rx(&mut self) -> bool {
        let mut queued = false;
        for _ in 0..(RX_END - RX_START) {
            self.write_reg(CR, CMD_PAGE1_START);
            let current = self.read_reg(CURR);
            self.write_reg(CR, CMD_START);
            let boundary = self.read_reg(BNRY);
            if !(RX_START..RX_END).contains(&current) || !(RX_START..RX_END).contains(&boundary) {
                self.initialize(InitializeMode::Running);
                return false;
            }
            let page = if boundary + 1 >= RX_END {
                RX_START
            } else {
                boundary + 1
            };
            if page == current {
                break;
            }
            let mut header = [0; 4];
            self.remote_read((page as u16) << 8, &mut header);
            let count = u16::from_le_bytes([header[2], header[3]]) as usize;
            let next = header[1];
            let pages = (RX_END - RX_START) as usize;
            let distance = if !(RX_START..RX_END).contains(&next) {
                0
            } else if next >= page {
                (next - page) as usize
            } else {
                (next as usize + pages) - page as usize
            };
            let expected = 1 + count / PAGE_BYTES;
            if !(RX_START..RX_END).contains(&next)
                || count < 4
                || distance == 0
                || (distance != expected && distance != expected + 1)
            {
                self.write_reg(
                    BNRY,
                    if current == RX_START {
                        RX_END - 1
                    } else {
                        current - 1
                    },
                );
                continue;
            }
            let length = count - 4;
            if (MIN_FRAME_LEN..=MAX_FRAME_LEN).contains(&length)
                && header[0] & 0x0f == 1
                && self.rx.len() < RX_QUEUE_LIMIT
                && let Ok(fresh) = FreshTxPacket::<LinkLayer>::alloc(length)
            {
                let mut bytes = [0u8; MAX_FRAME_LEN];
                self.read_ring(((page as u16) << 8) + 4, &mut bytes[..length]);
                let mut builder = fresh.to_builder();
                builder
                    .append_writer()
                    .write(&mut VmReader::from(&bytes[..length]));
                builder.commit(length);
                // This allocation has no DMA mapping, so conversion always succeeds.
                let packet = builder.build().to_rx_packet().unwrap();
                self.rx.push_back(packet);
                queued = true;
            }
            self.write_reg(
                BNRY,
                if next == RX_START {
                    RX_END - 1
                } else {
                    next - 1
                },
            );
        }
        queued
    }

    fn service(&mut self) -> (bool, bool) {
        if !self.running {
            return (false, false);
        }
        let generation = self.generation;
        let mut rx = false;
        let mut tx = false;
        for _ in 0..12 {
            self.write_reg(CR, CMD_START);
            let mut status = self.read_reg(ISR) & IRQ_MASK;
            if status == 0 {
                break;
            }
            // Acknowledge before draining: a newly arrived frame must leave
            // a fresh RX status bit rather than being erased after the drain.
            self.write_reg(ISR, status & RX_IRQ);
            if status & OVER != 0 {
                self.write_reg(CR, CMD_STOP);
                let until = ostd::arch::read_tsc().wrapping_add(ostd::arch::tsc_freq() / 100);
                while ostd::arch::read_tsc().wrapping_sub(until) > u64::MAX / 2 {
                    core::hint::spin_loop();
                }
                // A transmission can complete while the controller stops.
                // Re-read the completion bits before deciding to retransmit.
                status |= self.read_reg(ISR) & TX_IRQ;
                let resend = self.active.is_some() && status & TX_IRQ == 0;
                self.write_reg(RBCR0, 0);
                self.write_reg(RBCR1, 0);
                self.write_reg(TCR, TCR_LOOPBACK);
                self.write_reg(CR, CMD_START);
                rx |= self.drain_rx();
                if self.generation != generation {
                    return (false, true);
                }
                self.write_reg(ISR, OVER);
                self.write_reg(TCR, 0);
                if resend {
                    self.write_reg(CR, CMD_TRANSMIT);
                }
            } else if status & RX_IRQ != 0 {
                rx |= self.drain_rx();
                if self.generation != generation {
                    return (false, true);
                }
            }
            if status & TX_IRQ != 0 {
                self.write_reg(ISR, status & TX_IRQ);
                self.tx_complete();
                tx = true;
            }
            if status & COUNTERS != 0 {
                let _ = (self.read_reg(TCR), self.read_reg(DCR), self.read_reg(IMR));
                self.write_reg(ISR, COUNTERS);
            }
        }
        rx |= self.drain_rx();
        (rx, tx || self.generation != generation)
    }

    fn watchdog(&mut self) -> bool {
        if self.active.is_some()
            && ostd::arch::read_tsc().wrapping_sub(self.tx_started) > ostd::arch::tsc_freq() / 5
        {
            self.write_reg(IMR, 0);
            if self.reset().is_ok() {
                self.initialize(InitializeMode::Running);
            } else {
                self.stop();
            }
            return true;
        }
        false
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.write_reg(IMR, 0);
        self.write_reg(ISR, ACK_ALL);
        self.write_reg(CR, CMD_STOP);
    }
}

/// A single RTL8029 Ethernet controller backed by programmed I/O.
///
/// Lock order: the network device lock precedes `state`. Hard IRQs and the
/// taskless job only acquire `state`, and notify the network after releasing it.
pub struct Ne2k {
    state: Arc<SpinLock<State, LocalIrqDisabled>>,
    mac: [u8; 6],
}
impl fmt::Debug for Ne2k {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ne2k").field("mac", &self.mac).finish()
    }
}

impl AnyNetworkDevice for Ne2k {
    fn mac_addr(&self) -> EthernetAddress {
        EthernetAddress(self.mac)
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MAX_TX_FRAME_LEN;
        caps.max_burst_size = Some(2);
        caps
    }
    fn can_receive(&self) -> bool {
        let state = self.state.lock();
        state.running && !state.rx.is_empty()
    }
    fn can_send(&self) -> bool {
        let mut state = self.state.lock();
        // Polling may emit several packets before softirqs run again.
        state.reap_tx();
        state.running && state.slots.iter().any(Option::is_none)
    }
    fn receive(&mut self) -> core::result::Result<RxPacket<LinkLayer>, NetError> {
        let mut state = self.state.lock();
        if !state.running {
            return Err(NetError::NotReady);
        }
        state.rx.pop_front().ok_or(NetError::NotReady)
    }
    fn send(&mut self, packet: TxPacket<LinkLayer>) -> core::result::Result<(), NetError> {
        let frame_len = packet.len();
        let mut state = self.state.lock();
        if !state.running {
            return Err(NetError::NotReady);
        }
        if frame_len > MAX_TX_FRAME_LEN {
            return Err(NetError::Busy);
        }
        state.reap_tx();
        let slot = state
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(NetError::Busy)?;
        let mut bytes = [0u8; MAX_TX_FRAME_LEN];
        let length = frame_len.max(MIN_FRAME_LEN);
        packet
            .reader()
            .read(&mut VmWriter::from(&mut bytes[..frame_len]));
        state.write_reg(IMR, 0);
        if state
            .remote_write(TX_START + slot as u8 * (TX_PAGES / 2), &bytes[..length])
            .is_err()
        {
            if state.reset().is_ok() {
                state.initialize(InitializeMode::Running);
            } else {
                state.stop();
            }
            return Err(NetError::NotReady);
        }
        state.slots[slot] = Some(length);
        if state.active.is_none() {
            state.trigger(slot);
        }
        state.write_reg(IMR, IRQ_MASK);
        Ok(())
    }
    fn alloc_tx_buffer(payload_len: usize) -> Result<FreshTxPacket> {
        FreshTxPacket::alloc(payload_len)
    }

    fn free_processed_tx_buffers(&mut self) {
        self.state.lock().reap_tx();
    }
    fn notify_poll_end(&mut self) {}
}

struct OwnedPci {
    // Drop the mapped interrupt before the remaining state owners.
    _irq: MappedIrqLine,
    common: PciCommonDevice,
    _taskless: Arc<Taskless>,
    _net: Arc<SpinLock<Ne2k, BottomHalfDisabled>>,
}

impl Drop for OwnedPci {
    fn drop(&mut self) {
        self._net.lock().state.lock().stop();
        self._taskless.disable();
    }
}

impl fmt::Debug for OwnedPci {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedNe2kPci")
            .field("id", self.common.device_id())
            .finish()
    }
}

impl PciDevice for OwnedPci {
    fn device_id(&self) -> PciDeviceId {
        *self.common.device_id()
    }
}

#[derive(Debug)]
struct Driver {
    claimed: AtomicBool,
}

impl Driver {
    fn probe_device(&self, common: &PciCommonDevice) -> Result<OwnedResources> {
        let Bar::Io(bar) = common.bar_manager().bar(0).ok_or(Error::InvalidArgs)? else {
            return Err(Error::InvalidArgs);
        };
        if bar.base() == 0 || bar.size() < IO_EXTENT || bar.base() > u16::MAX as u32 - IO_EXTENT {
            return Err(Error::InvalidArgs);
        }
        let line = common
            .location()
            .read8(PciCommonCfgOffset::InterruptLine as u16);
        if line == 0 || line > 15 {
            return Err(Error::InvalidArgs);
        }
        let ports = Ports::acquire(bar.base() as u16)?;
        let state = Arc::new(SpinLock::<State, LocalIrqDisabled>::new(State {
            ports,
            mac: [0; 6],
            running: false,
            generation: 0,
            rx: VecDeque::with_capacity(RX_QUEUE_LIMIT),
            slots: [None, None],
            active: None,
            tx_started: 0,
        }));
        {
            let mut state = state.lock();
            if state.read_reg(CR) == 0xff {
                return Err(Error::IoError);
            }
            state.reset()?;
            state.mac = state.read_prom();
            if state.mac == [0; 6] || state.mac[0] & 1 != 0 {
                return Err(Error::IoError);
            }
            state.initialize(InitializeMode::Stopped);
        }

        let deferred_state = state.clone();
        let taskless = Taskless::new(move || {
            let (rx, tx) = {
                let mut state = deferred_state.lock();
                // Poll completion first, so a lost IRQ does not cause an
                // already completed transmission to be treated as timed out.
                let (rx, tx) = state.service();
                let recovered = state.watchdog();
                if state.running {
                    state.write_reg(IMR, IRQ_MASK);
                }
                (rx, tx || recovered)
            };
            if rx {
                aster_network::raise_receive_softirq();
            }
            if tx {
                aster_network::raise_send_softirq();
            }
        });
        let mut irq = IRQ_CHIP
            .get()
            .ok_or(Error::IoError)?
            .map_isa_pin_to(IrqLine::alloc()?, line)?;
        let callback_state = state.clone();
        let callback_job = taskless.clone();
        irq.on_active(move |_| {
            // Never allocate or release packet storage in hard-IRQ context.
            {
                let state = callback_state.lock();
                state.write_reg(CR, CMD_START);
                state.write_reg(IMR, 0);
            }
            callback_job.schedule();
        });
        let mac = state.lock().mac;
        let net = Arc::new(SpinLock::new(Ne2k {
            state: state.clone(),
            mac,
        }));

        // The timer also schedules completion polling when an interrupt is
        // lost. Weak references avoid retaining a removed device's resources.
        let timer_job = Arc::downgrade(&taskless);
        ostd::timer::register_callback_on_cpu(move || {
            // Periodic polling also covers a lost RX interrupt or a drain
            // that exhausted its budget while packets continued to arrive.
            if ostd::timer::Jiffies::elapsed().as_u64().is_multiple_of(10)
                && let Some(job) = timer_job.upgrade()
            {
                job.schedule();
            }
        });
        common.write_command(
            (common.read_command() | Command::IO_SPACE)
                - (Command::INTERRUPT_DISABLE | Command::BUS_MASTER),
        );
        state.lock().initialize(InitializeMode::Running);
        Ok(OwnedResources { net, irq, taskless })
    }
}

struct OwnedResources {
    net: Arc<SpinLock<Ne2k, BottomHalfDisabled>>,
    irq: MappedIrqLine,
    taskless: Arc<Taskless>,
}

impl PciDriver for Driver {
    fn probe(
        &self,
        common: PciCommonDevice,
    ) -> core::result::Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        let id = common.device_id();
        if id.vendor_id != REALTEK_VENDOR_ID || id.device_id != RTL8029_DEVICE_ID {
            return Err((BusProbeError::DeviceNotMatch, common));
        }
        if self
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err((BusProbeError::DeviceNotMatch, common));
        }
        match self.probe_device(&common) {
            Ok(OwnedResources { net, irq, taskless }) => {
                aster_network::register_device(super::DEVICE_NAME.to_string(), net.clone());
                Ok(Arc::new(OwnedPci {
                    _irq: irq,
                    common,
                    _taskless: taskless,
                    _net: net,
                }))
            }
            Err(error) => {
                self.claimed.store(false, Ordering::Release);
                ostd::warn!("RTL8029 probe failed: {:?}", error);
                Err((BusProbeError::ConfigurationSpaceError, common))
            }
        }
    }
}

pub(super) fn register() {
    PCI_BUS.lock().register_driver(Arc::new(Driver {
        claimed: AtomicBool::new(false),
    }));
}
