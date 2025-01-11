// SPDX-License-Identifier: MPL-2.0

use ostd_pod::Pod;

use super::{Iv, Key, Mac, VersionId};
use crate::{
    layers::bio::{BlockSet, Buf, BLOCK_SIZE},
    os::{Aead, Mutex},
    prelude::*,
};

/// A cryptographically-protected blob of user data.
///
/// `CryptoBlob<B>` allows a variable-length of user data to be securely
/// written to and read from a fixed, pre-allocated block set
/// (represented by `B: BlockSet`) on disk. Obviously, the length of user data
/// must be smaller than that of the block set.
///
/// # On-disk format
///
/// The on-disk format of `CryptoBlob` is shown below.
///
/// ```
/// ┌─────────┬─────────┬─────────┬──────────────────────────────┐
/// │VersionId│   MAC   │  Length │       Encrypted Payload      │
/// │  (8B)   │  (16B)  │   (8B)  │        (Length bytes)        │
/// └─────────┴─────────┴─────────┴──────────────────────────────┘
/// ```
///
/// The version ID increments by one each time the `CryptoBlob` is updated.
/// The MAC protects the integrity of the length and the encrypted payload.
///
/// # Security
///
/// To ensure the confidentiality and integrity of user data, `CryptoBlob`
/// takes several measures:
///
/// 1. Each instance of `CryptoBlob` is associated with a randomly-generated,
///    unique encryption key.
/// 2. Each instance of `CryptoBlob` maintains a version ID, which is
///    automatically incremented by one upon each write.
/// 3. The user data written to a `CryptoBlob` is protected with authenticated
///    encryption before being persisted to the disk.
///    The encryption takes the current version ID as the IV and generates a MAC
///    as the output.
/// 4. To read user data from a `CryptoBlob`, it first decrypts
///    the untrusted on-disk data with the encryption key associated with this object
///    and validating its integrity. Optimally, the user can check the version ID
///    of the decrypted user data and see if the version ID is up-to-date.
///
pub struct CryptoBlob<B> {
    block_set: B,
    key: Key,
    header: Mutex<Option<Header>>,
}

#[repr(C)]
#[derive(Copy, Clone, Pod)]
struct Header {
    version: VersionId,
    mac: Mac,
    payload_len: usize,
}

impl<B: BlockSet> CryptoBlob<B> {
    /// The size of the header of a crypto blob in bytes.
    pub const HEADER_NBYTES: usize = core::mem::size_of::<Header>();

    /// Opens an existing `CryptoBlob`.
    ///
    /// The capacity of this `CryptoBlob` object is determined by the size
    /// of `block_set: B`.
    pub fn open(key: Key, block_set: B) -> Self {
        Self {
            block_set,
            key,
            header: Mutex::new(None),
        }
    }

    /// Creates a new `CryptoBlob`.
    ///
    /// The encryption key of a `CryptoBlob` is generated randomly so that
    /// no two `CryptoBlob` instances shall ever use the same key.
    pub fn create(block_set: B, init_data: &[u8]) -> Result<Self> {
        let capacity = block_set.nblocks() * BLOCK_SIZE - Self::HEADER_NBYTES;
        if init_data.len() > capacity {
            return_errno_with_msg!(OutOfDisk, "init_data is too large");
        }
        let nblocks = (Self::HEADER_NBYTES + init_data.len()).div_ceil(BLOCK_SIZE);
        let mut block_buf = Buf::alloc(nblocks)?;

        // Encrypt init_data.
        let aead = Aead::new();
        let key = Key::random();
        let version: VersionId = 0;
        let mut iv = Iv::new_zeroed();
        iv.as_bytes_mut()[..version.as_bytes().len()].copy_from_slice(version.as_bytes());
        let output = &mut block_buf.as_mut_slice()
            [Self::HEADER_NBYTES..Self::HEADER_NBYTES + init_data.len()];
        let mac = aead.encrypt(init_data, &key, &iv, &[], output)?;

        // Store header.
        let header = Header {
            version,
            mac,
            payload_len: init_data.len(),
        };
        block_buf.as_mut_slice()[..Self::HEADER_NBYTES].copy_from_slice(header.as_bytes());

        // Write to `BlockSet`.
        block_set.write(0, block_buf.as_ref())?;
        Ok(Self {
            block_set,
            key,
            header: Mutex::new(Some(header)),
        })
    }

    /// Write the buffer to the disk as the latest version of the content of
    /// this `CryptoBlob`.
    ///
    /// The size of the buffer must not be greater than the capacity of this
    /// `CryptoBlob`.
    ///
    /// Each successful write increments the version ID by one. If
    /// there is no valid version ID, an `Error` will be returned.
    /// User could get a version ID, either by a successful call to
    /// `read`, or `recover_from` another valid `CryptoBlob`.
    ///
    /// # Security
    ///
    /// This content is guaranteed to be confidential as long as the key is not
    /// known to an attacker.
    pub fn write(&mut self, buf: &[u8]) -> Result<VersionId> {
        if buf.len() > self.capacity() {
            return_errno_with_msg!(OutOfDisk, "write data is too large");
        }
        let nblocks = (Self::HEADER_NBYTES + buf.len()).div_ceil(BLOCK_SIZE);
        let mut block_buf = Buf::alloc(nblocks)?;

        // Encrypt payload.
        let aead = Aead::new();
        let version = match self.version_id() {
            Some(version) => version + 1,
            None => return_errno_with_msg!(NotFound, "write with no valid version ID"),
        };
        let mut iv = Iv::new_zeroed();
        iv.as_bytes_mut()[..version.as_bytes().len()].copy_from_slice(version.as_bytes());
        let output =
            &mut block_buf.as_mut_slice()[Self::HEADER_NBYTES..Self::HEADER_NBYTES + buf.len()];
        let mac = aead.encrypt(buf, &self.key, &iv, &[], output)?;

        // Store header.
        let header = Header {
            version,
            mac,
            payload_len: buf.len(),
        };
        block_buf.as_mut_slice()[..Self::HEADER_NBYTES].copy_from_slice(header.as_bytes());

        // Write to `BlockSet`.
        self.block_set.write(0, block_buf.as_ref())?;
        *self.header.lock() = Some(header);
        Ok(version)
    }

    /// Read the content of the `CryptoBlob` from the disk into the buffer.
    ///
    /// The given buffer must has a length that is no less than the size of
    /// the plaintext content of this `CryptoBlob`.
    ///
    /// # Security
    ///
    /// This content, including its length, is guaranteed to be authentic.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let header = match *self.header.lock() {
            Some(header) => header,
            None => {
                let mut header = Header::new_zeroed();
                self.block_set.read_slice(0, header.as_bytes_mut())?;
                header
            }
        };
        if header.payload_len > self.capacity() {
            return_errno_with_msg!(OutOfDisk, "payload_len is greater than the capacity");
        }
        if header.payload_len > buf.len() {
            return_errno_with_msg!(OutOfDisk, "read_buf is too small");
        }
        let nblock = (Self::HEADER_NBYTES + header.payload_len).div_ceil(BLOCK_SIZE);
        let mut block_buf = Buf::alloc(nblock)?;
        self.block_set.read(0, block_buf.as_mut())?;

        // Decrypt payload.
        let aead = Aead::new();
        let version = header.version;
        let mut iv = Iv::new_zeroed();
        iv.as_bytes_mut()[..version.as_bytes().len()].copy_from_slice(version.as_bytes());
        let input =
            &block_buf.as_slice()[Self::HEADER_NBYTES..Self::HEADER_NBYTES + header.payload_len];
        let output = &mut buf[..header.payload_len];
        aead.decrypt(input, &self.key, &iv, &[], &header.mac, output)?;
        *self.header.lock() = Some(header);
        Ok(header.payload_len)
    }

    /// Returns the key associated with this `CryptoBlob`.
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Returns the current version ID.
    ///
    /// # Security
    ///
    /// It is valid after a successful call to `create`, `read` or `write`.
    /// User could also get a version ID from another valid `CryptoBlob`,
    /// (usually a backup), through method `recover_from`.
    pub fn version_id(&self) -> Option<VersionId> {
        self.header.lock().map(|header| header.version)
    }

    /// Recover from another `CryptoBlob`.
    ///
    /// If `CryptoBlob` doesn't have a valid version ID, e.g., payload decryption
    /// failed when `read`, user could call this method to recover version ID and
    /// payload from another `CryptoBlob` (usually a backup).
    pub fn recover_from(&mut self, other: &CryptoBlob<B>) -> Result<()> {
        if self.capacity() != other.capacity() {
            return_errno_with_msg!(InvalidArgs, "capacity not aligned, recover failed");
        }
        if self.header.lock().is_some() {
            return_errno_with_msg!(InvalidArgs, "no need to recover");
        }
        let nblocks = self.block_set.nblocks();
        // Read version ID and payload from another `CryptoBlob`.
        let mut read_buf = Buf::alloc(nblocks)?;
        let payload_len = other.read(read_buf.as_mut_slice())?;
        let version = other.version_id().unwrap();

        // Encrypt payload.
        let aead = Aead::new();
        let mut iv = Iv::new_zeroed();
        iv.as_bytes_mut()[..version.as_bytes().len()].copy_from_slice(version.as_bytes());
        let input = &read_buf.as_slice()[..payload_len];
        let mut write_buf = Buf::alloc(nblocks)?;
        let output =
            &mut write_buf.as_mut_slice()[Self::HEADER_NBYTES..Self::HEADER_NBYTES + payload_len];
        let mac = aead.encrypt(input, self.key(), &iv, &[], output)?;

        // Store header.
        let header = Header {
            version,
            mac,
            payload_len,
        };
        write_buf.as_mut_slice()[..Self::HEADER_NBYTES].copy_from_slice(header.as_bytes());

        // Write to `BlockSet`.
        self.block_set.write(0, write_buf.as_ref())?;
        *self.header.lock() = Some(header);
        Ok(())
    }

    /// Returns the current MAC of encrypted payload.
    ///
    /// # Security
    ///
    /// It is valid after a successful call to `create`, `read` or `write`.
    pub fn current_mac(&self) -> Option<Mac> {
        self.header.lock().map(|header| header.mac)
    }

    /// Returns the capacity of this `CryptoBlob` in bytes.
    pub fn capacity(&self) -> usize {
        self.block_set.nblocks() * BLOCK_SIZE - Self::HEADER_NBYTES
    }

    /// Returns the number of blocks occupied by the underlying `BlockSet`.
    pub fn nblocks(&self) -> usize {
        self.block_set.nblocks()
    }
}

#[cfg(test)]
mod tests {
    use super::CryptoBlob;
    use crate::layers::bio::{BlockSet, MemDisk, BLOCK_SIZE};

    #[test]
    fn create() {
        let disk = MemDisk::create(2).unwrap();
        let init_data = [1u8; BLOCK_SIZE];
        let blob = CryptoBlob::create(disk, &init_data).unwrap();

        println!("blob key: {:?}", blob.key());
        assert_eq!(blob.version_id(), Some(0));
        assert_eq!(blob.nblocks(), 2);
        assert_eq!(
            blob.capacity(),
            2 * BLOCK_SIZE - CryptoBlob::<MemDisk>::HEADER_NBYTES
        );
    }

    #[test]
    fn open_and_read() {
        let disk = MemDisk::create(4).unwrap();
        let key = {
            let subset = disk.subset(0..2).unwrap();
            let init_data = [1u8; 1024];
            let blob = CryptoBlob::create(subset, &init_data).unwrap();
            blob.key
        };

        let subset = disk.subset(0..2).unwrap();
        let blob = CryptoBlob::open(key, subset);
        assert_eq!(blob.version_id(), None);
        assert_eq!(blob.nblocks(), 2);
        let mut buf = [0u8; BLOCK_SIZE];
        let payload_len = blob.read(&mut buf).unwrap();
        assert_eq!(buf[..payload_len], [1u8; 1024]);
    }

    #[test]
    fn write() {
        let disk = MemDisk::create(2).unwrap();
        let init_data = [0u8; BLOCK_SIZE];
        let mut blob = CryptoBlob::create(disk, &init_data).unwrap();

        let write_buf = [1u8; 1024];
        blob.write(&write_buf).unwrap();
        let mut read_buf = [0u8; 1024];
        blob.read(&mut read_buf).unwrap();
        assert_eq!(read_buf, [1u8; 1024]);
        assert_eq!(blob.version_id(), Some(1));
    }

    #[test]
    fn recover_from() {
        let disk = MemDisk::create(2).unwrap();
        let init_data = [1u8; 1024];
        let subset0 = disk.subset(0..1).unwrap();
        let mut blob0 = CryptoBlob::create(subset0, &init_data).unwrap();
        assert_eq!(blob0.version_id(), Some(0));
        blob0.write(&init_data).unwrap();
        assert_eq!(blob0.version_id(), Some(1));

        let subset1 = disk.subset(1..2).unwrap();
        let mut blob1 = CryptoBlob::open(blob0.key, subset1);
        assert_eq!(blob1.version_id(), None);
        blob1.recover_from(&blob0).unwrap();
        let mut read_buf = [0u8; BLOCK_SIZE];
        let payload_len = blob1.read(&mut read_buf).unwrap();
        assert_eq!(read_buf[..payload_len], [1u8; 1024]);
        assert_eq!(blob1.version_id(), Some(1));
    }
}
