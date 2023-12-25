# Case Study 2: Virtio devices on PCI bus

In our journal towards writing an OS without _unsafe_ Rust, a key obstacle is dealing with device drivers. Device drivers are the single largest contributor to OS complexity. In Linux, they constitute 70% of the code base. And due to their low-level nature, device driver code usually involves privileged tasks, like doing PIO or MMIO, accessing registers, registering interrupt handlers, etc. So the question is: can we figure out the right abstractions for the OS core to enable writing most driver code in unprivileged Rust?

Luckily, the answer is YES. And this document will explain why.

We will focus on Virtio devices on PCI bus. The reason is two-fold. First, Virtio devices are the single most important class of devices for our target usage, VM-based TEEs. Second, PCI bus is the most important bus for x86 architecture. Given the versatility of Virtio and the complexity of PCI bus, if a solution can work with Virtio devices on PCI, then it is most likely to work with other types of devices or buses.

## The problem

Here are some of the elements in PCI-based Virtio devices that may involve `unsafe` Rust.
* Access PCI configuration space (doing PIO with `in`/`out` instructions)
* Access PCI capabilities (specified by raw pointers calculated from BAR + offset)
* Initialize Virtio devices (doing MMIO with raw pointers)
* Allocate and initialize Virtio queues (managing physical pages)
* Push/pop entries to/from Virtio queues (accessing physical memory with raw pointers)

## The solution

### PCI bus

### Privileged part

```rust
// file: aster-core-libs/pci-io-port/lib.rs
use x86::IoPort;

/// The I/O port to write an address in the PCI 
/// configuration space.
pub const PCI_ADDR_PORT: IoPort<u32> = {
    // SAFETY. Write to this I/O port won't affect 
    // any typed memory.
    unsafe {
        IoPort::new(0x0cf8, Rights![Wr])
    }
}

/// The I/O port to read/write a value from the
/// PCI configuration space.
pub const PCI_DATA_PORT: IoPort<u32> = {
    // SAFETY. Read/write to this I/O port won't affect 
    // any typed memory.
    unsafe {
        IoPort::new(0x0cf8 + 0x04, Rights![Rd, Wr])
    }
};
```

### Unprivileged part

```rust
// file: aster-comps/pci/lib.rs
use pci_io_port::{PCI_ADDR_PORT, PCI_DATA_PORT};

/// The PCI configuration space, which enables the discovery,
/// initialization, and configuration of PCI devices.
pub struct PciConfSpace;

impl PciConfSpace {
    pub fn read_u32(bus: u8, slot: u8, offset: u32) -> u32 {
        let addr = (1 << 31) | 
            ((bus as u32) << 16) |
            ((slot as u32) << 11) |
            (offset & 0xFF);
        PCI_ADDR_PORT.write(addr);
        PCI_DATA_PORT.read()
    }

    pub fn write_u32(bus: u8, slot: u8, offset: u32, val: u32) -> u32 {
        let addr = (1 << 31) | 
            ((bus as u32) << 16) |
            ((slot as u32) << 11) |
            (offset & 0xFF);
        PCI_ADDR_PORT.write(addr);
        PCI_DATA_PORT.write(val)
    }

    pub fn probe_device(&self, bus: u8, slot: u8) -> Option<PciDeviceConfig> {
        todo!("omitted...")
    }
}

/// A scanner of PCI bus to probe all PCI devices.
pub struct PciScanner {
    bus_no: u8,
    slot: u8,
}

impl Iterator for PciScanner {
    type Item = PciDevice;
    
    fn next(&mut self) -> Option<Self::Item> {
        while !(self.bus_no == 255 && self.slot == 31) {
            if self.slot == 31 {
                self.bus_no += 1;
                self.slot = 0;
            }

            let config = PciConfSpace::probe_device(self.bus_no, self.slot);
            let slot = self.slot;
            self.slot += 1;

            if let Some(config) = config {
                todo!("convert the config to a device...")
            }
        }      
    }
}

/// A general PCI device
pub struct PciDevice {
    // ...
}

/// The configuration of a general PCI device.
pub struct PciDeviceConfig {
    // ...
}

/// The capabilities of a PCI device.
pub struct PciCapabilities {
    // ...
}
```

###  Virtio

Most code of Virtio drivers can be unprivileged thanks to the abstractions of `VmPager` and `VmCell` provided by the OS core.

```rust
// file: aster-comp-libs/virtio/transport.rs

/// The transport layer for configuring a Virtio device.
pub struct VirtioTransport {
    isr_cell: VmCell<u8>,
    // ...
}

impl VirtioTransport {
    /// Create a new instance.
    ///
    /// According to Virtio spec, the transport layer for
    /// configuring a Virtio device consists of four parts:
    ///
    /// * Common configuration structure
    /// * Notification structure
    /// * Interrupt Status Register (ISR)
    /// * Device-specific configuration structure
    ///
    /// This constructor requires four pointers to these parts.
    pub fn new(
        common_cfg_ptr: PAddr<CommonCfg>,
        isr_ptr: PAddr<u8>,
        notifier: PAddr<Notifier>,
        device_cfg: PAddr<DeviceCfg>, 
    ) -> Result<Self> {
        let isr_cell = Self::new_part(isr_ptr)?;
        todo!("do more initialization...")
    }

    /// Write ISR.
    pub fn write_isr(&self, new_val: u8) {
        self.isr_cell.write(new_val).unwrap()
    }

    /// Read ISR.
    pub fn read_isr(&self) -> u8 {
        self.isr_cell.read().unwrap()
    }

    fn new_part<T: Pod>(part: PAddr<T>) -> Result<VmCell<T>> {
        let addr = part.as_ptr() as usize;
        let page_addr = align_down(addr, PAGE_SIZE);
        let page_offset = addr % PAGE_SIZE;

        // Acquire the access to the physical page 
        // that contains the part. If the physical page
        // is not safe to access, e.g., when the page
        // has been used by the kernel, then the acquisition
        // will fail.
        let vm_pager = VmPagerOption::new(PAGE_SIZE)
            .paddr(page_addr)
            .exclusive(false)
            .build()?;
        let vm_cell = vm_pager.new_cell(page_offset)?;
        vm_cell
    }
}
```
