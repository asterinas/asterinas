#![no_std]

use alloc::string::String;
extern crate alloc;

/// Token delimiter marker
pub const TOKEN_MARKER: u8 = 0xFF;
/// Length bytes for compressed symbol
pub(crate) const LENGTH_BYTES: usize = 2;

pub const KSYM_NAME_LEN: usize = 1024;
/// Length bytes for symbol type
const TY_LEN: usize = 1;

const PREFIX_LEN: usize = LENGTH_BYTES + TY_LEN;
/// Mapped kallsyms structure from binary blob
pub struct KallsymsMapped<'a> {
    token_table: &'a [u8],
    token_index: &'a [u32],
    kallsyms_names: &'a [u8],
    kallsyms_offsets: &'a [u32],
    kallsyms_seqs_of_names: &'a [u32],
    kallsyms_addresses: &'a [u64],
    kallsyms_num_syms: usize,
    stext: u64,
    etext: u64,
}

impl<'a> KallsymsMapped<'a> {
    /// Convert binary data into the blob structure and return KallsymsMapped.
    /// # Safety
    /// The input blob must be well-formed and valid.
    /// Undefined behavior may occur if the blob is malformed.
    /// # WARNING
    /// The blob is expected to be page-aligned in memory.
    pub fn from_blob(blob: &'a [u8], stext: u64, etext: u64) -> Result<Self, &'static str> {
        // page-aligned by loader (4K)
        let base = blob.as_ptr() as usize;
        let mut off = 0usize;

        let align_off = |align: usize, off: usize| {
            let addr = base + off;
            let addr = (addr + align - 1) & !(align - 1);
            addr - base
        };

        // read num_syms (u64)
        if off + 8 > blob.len() {
            return Err("The number of symbols is missing");
        }
        let num_syms = u64::from_le_bytes(blob[off..off + 8].try_into().unwrap()) as usize;
        off += 8;

        // addresses [u64], align 8
        off = align_off(8, off);
        let need = num_syms * core::mem::size_of::<u64>();
        if off + need > blob.len() {
            return Err("The addresses array is missing");
        }
        let addresses =
            unsafe { core::slice::from_raw_parts(blob[off..].as_ptr() as *const u64, num_syms) };
        off += need;

        // offsets [u32], align 4
        off = align_off(4, off);
        let need = num_syms * core::mem::size_of::<u32>();
        if off + need > blob.len() {
            return Err("The offsets array is missing");
        }
        let offsets =
            unsafe { core::slice::from_raw_parts(blob[off..].as_ptr() as *const u32, num_syms) };
        off += need;

        // seqs [u32], align 4
        off = align_off(4, off);
        let need = num_syms * core::mem::size_of::<u32>();
        if off + need > blob.len() {
            return Err("The seqs array is missing");
        }
        let seqs =
            unsafe { core::slice::from_raw_parts(blob[off..].as_ptr() as *const u32, num_syms) };
        off += need;

        // names (len u64 + bytes), align 8
        off = align_off(8, off);
        if off + 8 > blob.len() {
            return Err("The names length is missing");
        }
        let names_len = u64::from_le_bytes(blob[off..off + 8].try_into().unwrap()) as usize;
        off += 8;
        if off + names_len > blob.len() {
            return Err("The names array is missing");
        }
        let names = &blob[off..off + names_len];
        off += names_len;

        // token table (len u64 + bytes), align 8
        off = align_off(8, off);
        if off + 8 > blob.len() {
            return Err("The token table length is missing");
        }
        let token_table_len = u64::from_le_bytes(blob[off..off + 8].try_into().unwrap()) as usize;
        off += 8;
        if off + token_table_len > blob.len() {
            return Err("The token table array is missing");
        }
        let token_table = &blob[off..off + token_table_len];
        off += token_table_len;

        // token index [u32] (len u64 + array), align 8 then 4
        off = align_off(8, off);
        if off + 8 > blob.len() {
            return Err("The token index length is missing");
        }
        let token_index_len = u64::from_le_bytes(blob[off..off + 8].try_into().unwrap()) as usize;
        off += 8;
        off = align_off(4, off);
        let need = token_index_len * core::mem::size_of::<u32>();
        if off + need > blob.len() {
            return Err("The token index array is missing");
        }
        let token_index = unsafe {
            core::slice::from_raw_parts(blob[off..].as_ptr() as *const u32, token_index_len)
        };
        // off += need;
        Ok(Self {
            token_table,
            token_index,
            kallsyms_names: names,
            kallsyms_offsets: offsets,
            kallsyms_seqs_of_names: seqs,
            kallsyms_addresses: addresses,
            kallsyms_num_syms: num_syms,
            stext,
            etext,
        })
    }

    /// Dump all symbols in address order and return as a string.
    /// Each line: 16-digit hex address + type + name.
    pub fn dump_all_symbols(&self) -> String {
        let mut out = String::new();
        let mut name_buf = [0u8; KSYM_NAME_LEN];

        for i in 0..self.kallsyms_num_syms {
            let addr = self.kallsyms_addresses[i];
            let start = self.kallsyms_offsets[i] as usize;
            let (len, ty) = Self::read_compressed_len_and_type(&self.kallsyms_names[start..]);
            let end = start + PREFIX_LEN + len as usize;
            let name =
                self.expand_symbol(&self.kallsyms_names[start + PREFIX_LEN..end], &mut name_buf);
            use core::fmt::Write as _;
            let _ = writeln!(out, "{:016x} {} {}", addr, ty as char, name);
        }
        out
    }

    /// Expand a compressed symbol data into the resulting uncompressed string,
    /// if uncompressed string is too long (>= maxlen), it will be truncated,
    /// given the offset to where the symbol is in the compressed stream.
    fn expand_symbol<'b>(&self, bytes: &[u8], name_buf: &'b mut [u8; KSYM_NAME_LEN]) -> &'b str {
        let mut i = 0;
        let mut offset = 0;
        if bytes[i] == TOKEN_MARKER {
            let mut token_id = None;
            // Try to parse 0xFF <id> 0xFF (1-byte) or 0xFF <id_hi> <id_lo> 0xFF (2-byte)
            if i + 2 < bytes.len() && bytes[i + 2] == TOKEN_MARKER {
                let id = bytes[i + 1] as u16;
                token_id = Some(id);
                i += 3;
            } else if i + 3 < bytes.len() && bytes[i + 3] == TOKEN_MARKER {
                let id = ((bytes[i + 1] as u16) << 8) | (bytes[i + 2] as u16);
                token_id = Some(id);
                i += 4;
            }
            if let Some(id) = token_id {
                if (id as usize) < self.token_index.len() {
                    let start = self.token_index[id as usize] as usize;
                    let end = if (id as usize) + 1 < self.token_index.len() {
                        self.token_index[(id as usize) + 1] as usize
                    } else {
                        self.token_table.len()
                    };
                    let token_str = &self.token_table[start..end];
                    let need_copy = (KSYM_NAME_LEN - offset).min(token_str.len());
                    name_buf[offset..offset + need_copy].copy_from_slice(&token_str[..need_copy]);
                    offset += need_copy;
                }
            }
            // Append the rest as raw bytes
            let need_copy = (KSYM_NAME_LEN - offset).min(bytes.len() - i);
            name_buf[offset..offset + need_copy].copy_from_slice(&bytes[i..i + need_copy]);
            offset += need_copy;
        } else {
            // Not a token; treat as raw bytes
            let need_copy = (KSYM_NAME_LEN - offset).min(bytes.len() - i);
            name_buf[offset..offset + need_copy].copy_from_slice(&bytes[i..i + need_copy]);
            offset += need_copy;
        }
        let name = core::str::from_utf8(&name_buf[..offset]).unwrap_or_default();
        name
    }

    fn read_compressed_len_and_type(bytes: &[u8]) -> (u16, char) {
        if bytes.len() < PREFIX_LEN {
            return (0, ' ');
        }
        // little-endian: lo first, then hi
        // we skip the type byte
        let len_lo = bytes[0 + TY_LEN] as u16;
        let len_hi = bytes[1 + TY_LEN] as u16;
        ((len_hi << 8) | len_lo, bytes[0] as char)
    }

    /// Address → Symbol lookup. Returns (symbol name, symbol size, offset within symbol, type) if found.
    ///
    /// See <https://elixir.bootlin.com/linux/v6.6/source/kernel/kallsyms.c#L446>
    pub fn lookup_address<'b>(
        &self,
        addr: u64,
        name_buf: &'b mut [u8; KSYM_NAME_LEN],
    ) -> Option<(&'b str, u64, u64, char)> {
        // Quick check: address within text section and symbols exist
        if addr < self.stext || addr >= self.etext || self.kallsyms_num_syms == 0 {
            return None;
        }
        let mut low = 0usize;
        let mut high = self.kallsyms_num_syms;

        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if self.kallsyms_addresses[mid] <= addr {
                low = mid;
            } else {
                high = mid;
            }
        }

        if low >= self.kallsyms_num_syms {
            return None;
        }

        // Search for the first aliased symbol. Aliased
        // symbols are symbols with the same address.
        while low > 0 && self.kallsyms_addresses[low - 1] == addr {
            low -= 1;
        }
        let symbol_start = self.kallsyms_addresses[low];

        let mut symbol_end = None;
        // Search for next non-aliased symbol.
        for high in (low + 1)..self.kallsyms_num_syms {
            if self.kallsyms_addresses[high] > symbol_start {
                symbol_end = Some(self.kallsyms_addresses[high]);
                break;
            }
        }
        // If we found no next symbol, we use the end of the section.
        let symbol_end = symbol_end.unwrap_or(self.etext);

        let start = self.kallsyms_offsets[low] as usize;
        let (len, ty) = Self::read_compressed_len_and_type(&self.kallsyms_names[start..]);
        let end = start + PREFIX_LEN + len as usize;
        Some((
            self.expand_symbol(&self.kallsyms_names[start + PREFIX_LEN..end], name_buf),
            symbol_end - symbol_start,
            addr - symbol_start,
            ty,
        ))
    }

    /// Lookup the address for this symbol. Returns 0 if not found.
    ///
    /// See <https://elixir.bootlin.com/linux/v6.6/source/kernel/kallsyms.c#L265>
    pub fn lookup_name(&self, name: &str) -> Option<u64> {
        let (start_idx, _end_idx) = self.lookup_names(name, false)?;
        let seq = self.kallsyms_seqs_of_names[start_idx] as usize;
        Some(self.kallsyms_addresses[seq])
    }

    /// Symbol name → Address lookup (binary search on name order). It returns the first and last
    /// name index (need_end) if there are multiple symbols with the same name. If need_end is false,
    /// only the first index is valid.
    ///
    /// See <https://elixir.bootlin.com/linux/v6.6/source/kernel/kallsyms.c#L208>
    pub fn lookup_names(&self, name: &str, need_end: bool) -> Option<(usize, usize)> {
        let mut name_buf = [0u8; KSYM_NAME_LEN];
        if self.kallsyms_num_syms == 0 {
            return None;
        }
        let mut low = 0usize;
        let mut high = self.kallsyms_num_syms - 1;
        let mut mid = 0usize;
        while low <= high {
            mid = low + (high - low) / 2;
            // address-order index
            let seq = self.kallsyms_seqs_of_names[mid] as usize;
            let start = self.kallsyms_offsets[seq] as usize;
            let (len, _ty) = Self::read_compressed_len_and_type(&self.kallsyms_names[start..]);
            let end = start + PREFIX_LEN + len as usize;
            let mid_name =
                self.expand_symbol(&self.kallsyms_names[start + PREFIX_LEN..end], &mut name_buf);
            match name.cmp(&mid_name) {
                core::cmp::Ordering::Equal => break,
                core::cmp::Ordering::Less => high = mid - 1,
                core::cmp::Ordering::Greater => low = mid + 1,
            }
        }
        if low > high {
            return None;
        }
        low = mid;
        // Check for earlier matches in case of duplicates
        while low > 0 {
            let seq = self.kallsyms_seqs_of_names[low - 1] as usize;
            let start = self.kallsyms_offsets[seq] as usize;
            let (len, _ty) = Self::read_compressed_len_and_type(&self.kallsyms_names[start..]);
            let end = start + PREFIX_LEN + len as usize;
            let mid_name =
                self.expand_symbol(&self.kallsyms_names[start + PREFIX_LEN..end], &mut name_buf);
            if mid_name != name {
                break;
            }
            low -= 1;
        }

        high = 0;
        if need_end {
            // Check for later matches in case of duplicates
            high = mid;
            while high < self.kallsyms_num_syms - 1 {
                let seq = self.kallsyms_seqs_of_names[high + 1] as usize;
                let start = self.kallsyms_offsets[seq] as usize;
                let (len, _ty) = Self::read_compressed_len_and_type(&self.kallsyms_names[start..]);
                let end = start + PREFIX_LEN + len as usize;
                let mid_name = self
                    .expand_symbol(&self.kallsyms_names[start + PREFIX_LEN..end], &mut name_buf);
                if mid_name != name {
                    break;
                }
                high += 1;
            }
        }

        return Some((low, high));
    }
}
