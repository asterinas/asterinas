// SPDX-License-Identifier: MPL-2.0

//! The EDID base block carried by the Linux boot protocol.
//!
//! [`EdidInfo`] preserves the boot-parameter ABI and validates display data
//! passed from EFI firmware or another bootloader to the kernel.

/// The length of an EDID base block, in bytes.
pub const EDID_BASE_BLOCK_SIZE: usize = 128;

/// The EDID base block at offset `0x140` in Linux boot parameters.
///
/// Bootloaders may leave this block empty or supply invalid data.
/// Accessors validate it before exposing display information.
/// Extension blocks do not fit in the boot protocol and are not used here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EdidInfo {
    bytes: [u8; EDID_BASE_BLOCK_SIZE],
}

impl EdidInfo {
    /// Copies a valid EDID 1.0 through 1.4 base block from the supplied bytes.
    ///
    /// Returns `None` for a short block, invalid header or checksum, or an
    /// unsupported version.
    /// Bytes after the base block are ignored.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..EDID_BASE_BLOCK_SIZE)?.try_into().ok()?;
        let info = Self { bytes };
        info.is_valid().then_some(info)
    }

    /// Returns the maximum display width and height in millimeters.
    ///
    /// Returns `None` if the base block is invalid or does not specify both
    /// dimensions.
    /// EDID's centimeter units limit the precision to 10 mm.
    pub fn physical_size_mm(&self) -> Option<(u16, u16)> {
        if !self.is_valid() {
            return None;
        }

        // VESA E-EDID 1.4, section 3.6.2: zero in either size byte indicates
        // an unspecified size or an aspect ratio, rather than two lengths.
        const WIDTH_CM_OFFSET: usize = 21;
        const HEIGHT_CM_OFFSET: usize = 22;
        let width_cm = self.bytes[WIDTH_CM_OFFSET];
        let height_cm = self.bytes[HEIGHT_CM_OFFSET];
        if width_cm == 0 || height_cm == 0 {
            return None;
        }

        const MILLIMETERS_PER_CENTIMETER: u16 = 10;
        Some((
            u16::from(width_cm) * MILLIMETERS_PER_CENTIMETER,
            u16::from(height_cm) * MILLIMETERS_PER_CENTIMETER,
        ))
    }

    fn is_valid(&self) -> bool {
        const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
        const VERSION_OFFSET: usize = 18;
        const REVISION_OFFSET: usize = 19;
        const SUPPORTED_VERSION: u8 = 1;
        const MAX_SUPPORTED_REVISION: u8 = 4;

        // EDID defines the base-block checksum as the byte sum modulo 256.
        let checksum = self
            .bytes
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        self.bytes.starts_with(&HEADER)
            && self.bytes[VERSION_OFFSET] == SUPPORTED_VERSION
            && self.bytes[REVISION_OFFSET] <= MAX_SUPPORTED_REVISION
            && checksum == 0
    }
}

impl Default for EdidInfo {
    fn default() -> Self {
        Self {
            bytes: [0; EDID_BASE_BLOCK_SIZE],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(width_cm: u8, height_cm: u8) -> [u8; EDID_BASE_BLOCK_SIZE] {
        let mut bytes = [0; EDID_BASE_BLOCK_SIZE];
        bytes[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        bytes[18] = 1;
        bytes[19] = 4;
        bytes[21] = width_cm;
        bytes[22] = height_cm;
        checksum(&mut bytes);
        bytes
    }

    fn checksum(bytes: &mut [u8; EDID_BASE_BLOCK_SIZE]) {
        bytes[EDID_BASE_BLOCK_SIZE - 1] = 0;
        bytes[EDID_BASE_BLOCK_SIZE - 1] =
            0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
    }

    #[test]
    fn reports_physical_dimensions_in_millimeters() {
        let bytes = block(52, 29);
        let info = EdidInfo::from_bytes(&bytes).unwrap();
        assert_eq!(info.physical_size_mm(), Some((520, 290)));
        assert_eq!(size_of::<EdidInfo>(), EDID_BASE_BLOCK_SIZE);
        assert_eq!(align_of::<EdidInfo>(), 1);
    }

    #[test]
    fn rejects_truncated_and_corrupt_blocks() {
        let mut bytes = block(52, 29);
        assert!(EdidInfo::from_bytes(&[]).is_none());
        assert!(EdidInfo::from_bytes(&bytes[..EDID_BASE_BLOCK_SIZE - 1]).is_none());
        bytes[21] += 1;
        assert!(EdidInfo::from_bytes(&bytes).is_none());
        assert_eq!(EdidInfo { bytes }.physical_size_mm(), None);

        bytes = block(52, 29);
        bytes[1] = 0;
        checksum(&mut bytes);
        assert!(EdidInfo::from_bytes(&bytes).is_none());
        assert_eq!(EdidInfo { bytes }.physical_size_mm(), None);
        assert_eq!(EdidInfo::default().physical_size_mm(), None);
    }

    #[test]
    fn rejects_unsupported_versions() {
        let mut bytes = block(52, 29);
        bytes[18] = 2;
        checksum(&mut bytes);
        assert!(EdidInfo::from_bytes(&bytes).is_none());
        bytes[18] = 1;
        bytes[19] = 5;
        checksum(&mut bytes);
        assert!(EdidInfo::from_bytes(&bytes).is_none());
    }

    #[test]
    fn does_not_treat_aspect_ratios_as_physical_dimensions() {
        for (width_cm, height_cm) in [(0, 0), (52, 0), (0, 29)] {
            let info = EdidInfo::from_bytes(&block(width_cm, height_cm)).unwrap();
            assert_eq!(info.physical_size_mm(), None);
        }
    }

    #[test]
    fn uses_only_the_base_block() {
        let mut bytes = block(52, 29);
        // The boot ABI does not carry extensions, even when the base lists one.
        bytes[126] = 1;
        checksum(&mut bytes);
        assert_eq!(
            EdidInfo::from_bytes(&bytes).unwrap().physical_size_mm(),
            Some((520, 290))
        );
        let mut with_extension = bytes.to_vec();
        with_extension.extend_from_slice(&[0xff; EDID_BASE_BLOCK_SIZE]);
        assert_eq!(
            EdidInfo::from_bytes(&with_extension)
                .unwrap()
                .physical_size_mm(),
            Some((520, 290))
        );
    }
}
