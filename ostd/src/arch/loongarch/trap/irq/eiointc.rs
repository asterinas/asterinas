// SPDX-License-Identifier: MPL-2.0

use loongArch64::iocsr::{iocsr_read_d, iocsr_write_b, iocsr_write_d, iocsr_write_h};

/// Reference: <http://github.com/loongson-community/docs/blob/master/3A5000/Loongson3A5000_3B5000%20user%20book_V1.3.pdf>
/// Section: 4.13
const OTHER_FUNCTION_SETTING_REG: usize = 0x420;

/// Extended I/O Interrupt Controller
///
/// In addition to being compatible with the original traditional I/O interrupt mode,
/// 3A5000 starts to support extended I/O interrupt,
/// which is used to directly distribute 256 bit interrupt on HT bus
/// to each processor core instead of forwarding through HT interrupt line,
/// so as to improve the flexibility of I/O interrupt use.
///
/// Reference: <http://github.com/loongson-community/docs/blob/master/3A5000/Loongson3A5000_3B5000%20user%20book_V1.3.pdf>
/// Section: 11.2
pub struct Eiointc;

impl Eiointc {
    const U64_BITS: usize = u64::BITS as _;
    const U64_BYTES: usize = u64::BITS as usize / 8;
    const MAX_CORE_NUM: usize = 4;
    const MAX_INTERRUPT_NUM: usize = 256;

    /// Extended I/O Interrupt Enable Register Base Address
    ///
    /// There are four 64 bit registers, and each bit of each register controls an interrupt.
    const EXT_IOI_EN_BASE: usize = 0x1600;
    /// Extended I/O Interrupt Bounce Register Base Address
    ///
    /// There are four 64 bit registers, and each bit of each register controls an interrupt.
    pub const EXT_IOI_BOUNCE_BASE: usize = 0x1680;
    /// Extended I/O Interrupt Status Register Base Address
    ///
    /// There are four 64 bit registers, and each bit of each register controls an interrupt.
    pub const EXT_IOI_SR_BASE: usize = 0x1700;
    /// Interrupt status of extended IO interrupt routed to current processor core
    ///
    /// There are four 64 bit registers, and each bit of each register controls an interrupt.
    pub const PER_CORE_EXT_IOI_SR_BASE: usize = 0x1800;
    /// Extended I/O Interrupt Pin Routing Register Base Address
    ///
    /// There are eight 8 bit registers.
    pub const EXT_IOI_MAP_BASE: usize = 0x14C0;
    const MAX_EXT_IOI_MAP_NUM: usize = 8;
    /// Interrupt destination processor core routing register base address
    ///
    /// There are 256 8 bit registers.
    pub const EXT_IOI_MAP_CORE_BASE: usize = 0x1C00;
    /// Interrupt target node mapping register base address
    ///
    /// There are 16 16 bit registers.
    pub const EXT_IOI_NODE_TYPE_BASE: usize = 0x14A0;

    /// Initialize extended I/O interrupt controller
    ///
    /// # Arguments
    ///
    /// * `core_num` - Number of processor cores
    ///
    /// # Note
    ///
    /// * Now only support single cpu, and the core num of cpu must less than or equal to 4
    pub fn init(core_num: usize) {
        assert!(core_num <= Self::MAX_CORE_NUM);
        // Enable extended I/O interrupt
        let mut v = iocsr_read_d(OTHER_FUNCTION_SETTING_REG);
        v |= 1 << 48;
        iocsr_write_d(OTHER_FUNCTION_SETTING_REG, v);
        // Set encoding method
        let mut v = iocsr_read_d(OTHER_FUNCTION_SETTING_REG);
        v |= 1 << 49;
        iocsr_write_d(OTHER_FUNCTION_SETTING_REG, v);
        // Set interrupt pin routing
        // 0..31    -> INT0
        // 32..63   -> INT1
        // 64..95   -> INT2
        // 96..127  -> INT3
        // 128..159 -> INT4
        // 160..191 -> INT5
        // 192..223 -> INT6
        // 224..255 -> INT7
        for i in 0..Self::MAX_EXT_IOI_MAP_NUM {
            iocsr_write_b(Self::EXT_IOI_MAP_BASE + i, i as _);
        }
        // Set the interrupt to bounce on the cores on node 0
        for i in 0..Self::MAX_INTERRUPT_NUM {
            iocsr_write_b(
                Self::EXT_IOI_MAP_CORE_BASE + i as usize,
                (1 << core_num) - 1,
            );
        }
        // Set the node type0 to node 0
        iocsr_write_h(Self::EXT_IOI_NODE_TYPE_BASE, 0x01);
        // Enable extended I/O interrupt
        let mut v = iocsr_read_d(OTHER_FUNCTION_SETTING_REG);
        v |= 1 << 48;
        iocsr_write_d(OTHER_FUNCTION_SETTING_REG, v);
    }

    /// Enable a specific IRQ.
    pub fn enable(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;
        // Enable the interrupt
        let mut enable = iocsr_read_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES);
        enable |= 1u64 << bit_offset;
        iocsr_write_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES, enable);
        // Enable the bounce
        let mut bounce = iocsr_read_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES);
        bounce |= 1u64 << bit_offset;
        iocsr_write_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES, bounce);
    }

    /// Disable a specific IRQ.
    pub fn disable(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;
        // Disable the interrupt
        let mut enable = iocsr_read_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES);
        enable &= !(1u64 << bit_offset);
        iocsr_write_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES, enable);
        // Disable the bounce
        let mut bounce = iocsr_read_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES);
        bounce &= !(1u64 << bit_offset);
        iocsr_write_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES, bounce);
    }

    /// Claim an interrupt.
    ///
    /// Return the active interrupt number if there is an active interrupt, otherwise return None.
    pub fn claim() -> Option<u8> {
        for i in 0..Self::MAX_INTERRUPT_NUM / Self::U64_BITS {
            let status = iocsr_read_d(Self::PER_CORE_EXT_IOI_SR_BASE + i * Self::U64_BYTES);
            if status != 0 {
                let irq = (i * Self::U64_BITS + Self::U64_BITS
                    - 1
                    - status.leading_zeros() as usize) as _;
                return Some(irq);
            }
        }
        None
    }

    /// Complete an interrupt
    pub fn complete(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;
        let mut status = iocsr_read_d(Self::EXT_IOI_SR_BASE + reg_no * Self::U64_BYTES);
        status &= !(1u64 << bit_offset);
        iocsr_write_d(Self::EXT_IOI_SR_BASE + reg_no * Self::U64_BYTES, status);
    }
}
