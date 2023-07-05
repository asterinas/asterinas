use core::iter;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PciDeviceLocation {
    pub bus: u8,
    /// Max 31
    pub device: u8,
    /// Max 7
    pub function: u8,
}

impl PciDeviceLocation {
    pub const MIN_BUS: u8 = 0;
    pub const MAX_BUS: u8 = 255;
    pub const MIN_DEVICE: u8 = 0;
    pub const MAX_DEVICE: u8 = 31;
    pub const MIN_FUNCTION: u8 = 0;
    pub const MAX_FUNCTION: u8 = 7;
    /// By encoding bus, device, and function into u32, user can access a PCI device in x86 by passing in this value.
    #[inline(always)]
    pub fn encode_as_x86_address_value(self) -> u32 {
        // 1 << 31: Configuration enable
        (1 << 31)
            | ((self.bus as u32) << 16)
            | (((self.device as u32) & 0b11111) << 11)
            | (((self.function as u32) & 0b111) << 8)
    }

    /// Returns an iterator that enumerates all possible PCI device locations.
    pub fn all() -> impl Iterator<Item = PciDeviceLocation> {
        iter::from_generator(|| {
            for bus in Self::MIN_BUS..=Self::MAX_BUS {
                for device in Self::MIN_DEVICE..=Self::MAX_DEVICE {
                    for function in Self::MIN_FUNCTION..=Self::MAX_FUNCTION {
                        let loc = PciDeviceLocation {
                            bus,
                            device,
                            function,
                        };
                        yield loc;
                    }
                }
            }
        })
    }
}
