// SPDX-License-Identifier: MPL-2.0

#[cfg(test)]
mod tests {
    use ostd_pod::AlignedBytes;
    use zerocopy::{FromZeros, IntoBytes};

    #[test]
    fn aligned_bytes_creation() {
        let bytes: AlignedBytes<u64, 8> = AlignedBytes::new_zeroed();
        assert_eq!(bytes.as_bytes().len(), 8);
        assert_eq!(bytes.as_bytes(), &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn aligned_bytes_mut() {
        let mut bytes: AlignedBytes<u32, 4> = AlignedBytes::new_zeroed();
        let slice = bytes.as_mut_bytes();
        slice[0] = 42;
        slice[1] = 100;
        slice[2] = 200;
        slice[3] = 255;

        assert_eq!(bytes.as_bytes()[0], 42);
        assert_eq!(bytes.as_bytes()[1], 100);
        assert_eq!(bytes.as_bytes()[2], 200);
        assert_eq!(bytes.as_bytes()[3], 255);
    }

    #[test]
    fn aligned_bytes_default() {
        let bytes: AlignedBytes<u64, 16> = AlignedBytes::default();
        assert_eq!(bytes.as_bytes().len(), 16);
        assert!(bytes.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn aligned_bytes_u32_alignment() {
        let bytes: AlignedBytes<u32, 8> = AlignedBytes::new_zeroed();
        let ptr = bytes.as_bytes().as_ptr() as usize;
        // u32 has alignment 4
        assert_eq!(ptr % align_of::<u32>(), 0);
    }

    #[test]
    fn aligned_bytes_u64_alignment() {
        let bytes: AlignedBytes<u64, 8> = AlignedBytes::new_zeroed();
        let ptr = bytes.as_bytes().as_ptr() as usize;
        // u64 has alignment 8
        assert_eq!(ptr % align_of::<u64>(), 0);
    }

    #[test]
    fn aligned_bytes_large_size() {
        let bytes: AlignedBytes<u64, 1024> = AlignedBytes::new_zeroed();
        assert_eq!(bytes.as_bytes().len(), 1024);
        assert!(bytes.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn aligned_bytes_write_and_read() {
        let mut bytes: AlignedBytes<u32, 16> = AlignedBytes::new_zeroed();

        // Write pattern
        for i in 0..16 {
            bytes.as_mut_bytes()[i] = (i * 17) as u8;
        }

        // Read and verify
        for i in 0..16 {
            assert_eq!(bytes.as_bytes()[i], (i * 17) as u8);
        }
    }

    #[test]
    fn aligned_bytes_zero_size() {
        let bytes: AlignedBytes<u64, 0> = AlignedBytes::new_zeroed();
        assert_eq!(bytes.as_bytes().len(), 0);
    }

    #[test]
    fn aligned_bytes_single_byte() {
        let mut bytes: AlignedBytes<u64, 1> = AlignedBytes::new_zeroed();
        bytes.as_mut_bytes()[0] = 255;
        assert_eq!(bytes.as_bytes()[0], 255);
    }
}
