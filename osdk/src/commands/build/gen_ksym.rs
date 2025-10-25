// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashMap, fmt::Debug};

const TOKEN_MARKER: u8 = 0xFF;

/// Candidate prefix lengths for heuristic tokenization
// const PREFIX_CANDIDATE_LENS: &[usize] = &[8, 16, 24, 32, 64, 96, 128, 512, 1024, 2048];
const PREFIX_CANDIDATE_LENS: &[usize] = &[
    10, 24, 31, 40, 56, 60, 70, 80, 90, 100, 150, 200, 250, 300, 400, 500, 600, 700, 800, 900,
    1000, 1200, 1400, 1600, 1800, 2000,
];

/// Maximum number of tokens
const MAX_TOKEN: usize = 512;

/// The structure for compressed symbol data
pub struct KallsymsBlob {
    pub token_table: Vec<u8>,
    /// The start index of each token in token_table
    pub token_index: Vec<u32>,
    pub token_map: HashMap<String, u16>,
    /// Compressed symbol data
    pub kallsyms_names: Vec<u8>,
    /// The offsets of each symbol in kallsyms_names
    pub kallsyms_offsets: Vec<u32>,
    /// The sequence numbers of each symbol
    pub kallsyms_seqs_of_names: Vec<u32>,
    /// The addresses of each symbol
    pub kallsyms_addresses: Vec<u64>,
    pub kallsyms_num_syms: usize,
}

impl Debug for KallsymsBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KallsymsBlob")
            .field("token_table size", &(self.token_table.len()))
            .field("token_index size", &(self.token_index.len() * 4))
            .field("kallsyms_names size", &(self.kallsyms_names.len()))
            .field("kallsyms_offsets size", &(self.kallsyms_offsets.len() * 4))
            .field(
                "kallsyms_seqs_of_names size",
                &(self.kallsyms_seqs_of_names.len() * 4),
            )
            .field(
                "kallsyms_addresses size",
                &(self.kallsyms_addresses.len() * 8),
            )
            .field("kallsyms_num_syms", &self.kallsyms_num_syms)
            .finish()
    }
}

impl KallsymsBlob {
    pub fn new() -> Self {
        Self {
            token_table: Vec::new(),
            token_index: Vec::new(),
            token_map: HashMap::new(),
            kallsyms_names: Vec::new(),
            kallsyms_offsets: Vec::new(),
            kallsyms_seqs_of_names: Vec::new(),
            kallsyms_addresses: Vec::new(),
            kallsyms_num_syms: 0,
        }
    }

    /// Add a token to the token table
    fn add_token(&mut self, token: String) -> Option<u16> {
        if let Some(&id) = self.token_map.get(&token) {
            return Some(id);
        }
        let id = self.token_map.len() as u16;
        self.token_index.push(self.token_table.len() as u32);
        self.token_table.extend_from_slice(token.as_bytes());
        self.token_map.insert(token, id);
        Some(id)
    }

    /// Compress all symbol names, auto-generating tokens.
    /// Storage order: address order, with a name-order to address-order mapping.
    pub fn compress_symbols(&mut self, symbols: &[(String, u64, char)]) {
        // 0) Build indices for name order and address order.
        let n = symbols.len();
        if n == 0 {
            return;
        }
        let mut idx_by_addr: Vec<usize> = (0..n).collect();
        idx_by_addr.sort_by_key(|&i| symbols[i].1);
        let mut idx_by_name: Vec<usize> = (0..n).collect();
        idx_by_name.sort_by(|&i, &j| symbols[i].0.cmp(&symbols[j].0));

        // map original index -> address-order index
        let mut orig_to_addr_idx = vec![0usize; n];
        for (addr_pos, &orig_idx) in idx_by_addr.iter().enumerate() {
            orig_to_addr_idx[orig_idx] = addr_pos;
        }

        // 1) Count possible tokens using prefix-based heuristic.
        // For each symbol, consider only prefixes of specific lengths.
        let mut token_count: HashMap<String, usize> = HashMap::new();
        for (name, _, _) in symbols.iter() {
            let bytes = name.as_bytes();
            for &len in PREFIX_CANDIDATE_LENS {
                if bytes.len() >= len {
                    let token = std::str::from_utf8(&bytes[..len]).unwrap();
                    *token_count.entry(token.to_string()).or_insert(0) += 1;
                } else if !bytes.is_empty() && len == PREFIX_CANDIDATE_LENS[0] {
                    // Edge case: name shorter than the smallest candidate length; include full name to avoid missing short common prefixes
                    let token = name;
                    *token_count.entry(token.to_string()).or_insert(0) += 1;
                }
            }
        }

        // 2) Select high-frequency tokens (cap at MAX_TOKEN), prefer longer tokens on tie
        let mut tokens: Vec<(String, usize)> = token_count.into_iter().collect();
        tokens.sort_by(|a, b| {
            // primary: frequency desc; secondary: length desc;
            // b.1.cmp(&a.1).then_with(|| b.0.len().cmp(&a.0.len()))
            (b.1 * b.0.len()).cmp(&(a.1 * a.0.len())) // weighted by length(more effective)
        });
        let mut final_token_list: Vec<String> = Vec::new();
        for (tok, _) in tokens.into_iter() {
            if final_token_list.len() >= MAX_TOKEN {
                break;
            }
            // Avoid tokens that are prefixes of existing tokens
            let mut is_prefix = false;
            for existing in &final_token_list {
                if existing.starts_with(&tok) {
                    is_prefix = true;
                    break;
                }
            }
            if !is_prefix {
                final_token_list.push(tok);
            }
        }
        for tok in final_token_list.into_iter() {
            self.add_token(tok);
        }

        // 3) Compress symbols in address order and build offsets/addresses.
        for &orig_idx in &idx_by_addr {
            let (ref sym, addr, ty) = symbols[orig_idx];
            self.kallsyms_offsets.push(self.kallsyms_names.len() as u32);
            self.kallsyms_addresses.push(addr);

            let sym_bytes = sym.as_bytes();
            // Only allow a single token at the beginning (prefix) according to heuristic.
            let rem = sym_bytes.len();
            let mut consumed = 0usize;
            for &l in PREFIX_CANDIDATE_LENS.iter().rev() {
                if l > rem {
                    // if the candidate length exceeds the remaining length, skip
                    continue;
                }
                let candidate = &sym[..l];
                if let Some(&id) = self.token_map.get(candidate) {
                    // 1. [type] [length] (0xff  token             0xff) [remaining bytes]
                    // 2. [type] [length] (0xff  token_hi token_lo 0xff) [remaining bytes]
                    //    [1byte][2bytes] (1byte 1byte    1byte   1byte) [remaining bytes]
                    let mut length: u16 = if id < 256 { 3 } else { 4 };
                    length += (rem - l) as u16;
                    // Emit type char
                    self.kallsyms_names.push(ty as u8);

                    // Emit length (little-endian: lo, hi)
                    self.kallsyms_names.push((length & 0xFF) as u8);
                    self.kallsyms_names.push((length >> 8) as u8);

                    // Emit token
                    self.kallsyms_names.push(TOKEN_MARKER);
                    if id < 256 {
                        self.kallsyms_names.push(id as u8);
                    } else {
                        self.kallsyms_names.push((id >> 8) as u8);
                        self.kallsyms_names.push((id & 0xFF) as u8);
                    }
                    self.kallsyms_names.push(TOKEN_MARKER);
                    consumed = l;
                    break;
                }
            }

            if consumed == 0 {
                // No token matched; emit full symbol as raw bytes
                // Emit type char
                self.kallsyms_names.push(ty as u8);
                // Emit length
                let length = rem as u16;
                // little-endian: lo, hi
                self.kallsyms_names.push((length & 0xFF) as u8);
                self.kallsyms_names.push((length >> 8) as u8);
            }

            // Emit remaining bytes raw
            self.kallsyms_names
                .extend_from_slice(&sym_bytes[consumed..]);
        }

        // 4) Build name-order -> address-order sequence mapping.
        self.kallsyms_seqs_of_names.reserve(n);
        for &orig_idx in &idx_by_name {
            let addr_idx = orig_to_addr_idx[orig_idx] as u32;
            self.kallsyms_seqs_of_names.push(addr_idx);
        }

        self.kallsyms_num_syms = n;
    }

    /// Serialize blob into bytes
    /// Convert the blob into binary data
    pub fn to_blob(&self) -> Vec<u8> {
        let mut blob = Vec::new();

        #[inline]
        fn pad(vec: &mut Vec<u8>, align: usize) {
            let rem = vec.len() % align;
            if rem != 0 {
                vec.resize(vec.len() + (align - rem), 0);
            }
        }

        blob.extend_from_slice(&(self.kallsyms_num_syms as u64).to_le_bytes());

        // addresses [u64]
        pad(&mut blob, 8);
        for &addr in &self.kallsyms_addresses {
            blob.extend_from_slice(&addr.to_le_bytes());
        }
        // offsets [u32]
        pad(&mut blob, 4);
        for &off in &self.kallsyms_offsets {
            blob.extend_from_slice(&(off).to_le_bytes());
        }
        // seqs [u32]
        pad(&mut blob, 4);
        for &seq in &self.kallsyms_seqs_of_names {
            blob.extend_from_slice(&seq.to_le_bytes());
        }

        // names bytes (len u64 + bytes)
        pad(&mut blob, 8);
        blob.extend_from_slice(&(self.kallsyms_names.len() as u64).to_le_bytes());
        blob.extend_from_slice(&self.kallsyms_names);

        // token table bytes (len u64 + bytes)
        pad(&mut blob, 8);
        blob.extend_from_slice(&(self.token_table.len() as u64).to_le_bytes());
        blob.extend_from_slice(&self.token_table);

        // token index [u32] (len u64 + array)
        pad(&mut blob, 8);
        blob.extend_from_slice(&(self.token_index.len() as u64).to_le_bytes());
        pad(&mut blob, 4);
        for &idx in &self.token_index {
            blob.extend_from_slice(&idx.to_le_bytes());
        }

        blob
    }
}

pub fn symbol_info(line: &str) -> Option<(String, u64, char)> {
    if line.len() > 4096 {
        panic!("The kernel symbol is too long: {}", line);
    }
    let mut parts = line.split_whitespace();
    let vaddr = u64::from_str_radix(parts.next()?, 16).ok()?;
    let symbol_type = parts.next()?.chars().next()?;
    let mut symbol = parts.collect::<Vec<_>>().join(" ");
    if symbol.starts_with("_ZN") {
        symbol = format!("{:#}", rustc_demangle::demangle(&symbol));
    } else {
        symbol = symbol.to_string();
    }
    Some((symbol, vaddr, symbol_type))
}

#[cfg(test)]
mod tests {
    const KSYM_NAME_LEN: usize = 1024;

    use super::KallsymsBlob;
    #[test]
    fn test() {
        let symbols = r#"
        0000000000001000 T do_mkdir
        0000000000001300 T alias_do_fork
        0000000000001300 T do_fork
        0000000000001300 T do_fork_2
        0000000000001200 T cpu_startup_entry
        0000000000001400 T cpu_startup_entry
    "#;
        let symbols: Vec<(String, u64, char)> = symbols
            .lines()
            .filter_map(|line| super::symbol_info(line.trim()))
            .collect();

        println!("Original symbols: {:?}", symbols);

        let mut blob = KallsymsBlob::new();
        blob.compress_symbols(&symbols);

        let binary_blob = blob.to_blob();
        println!("Binary blob size: {} bytes", binary_blob.len());

        let mapped =
            ksym_bin::KallsymsMapped::from_blob(&binary_blob, 0x1000, 0x1500).expect("parse blob");

        assert_eq!(mapped.lookup_address(100, &mut [0; KSYM_NAME_LEN]), None);
        assert_eq!(
            mapped.lookup_address(0x1200, &mut [0; KSYM_NAME_LEN]),
            Some(("cpu_startup_entry", 0x100, 0, 'T'))
        );
        // Test aliased symbols
        assert_eq!(
            mapped.lookup_address(0x1300, &mut [0; KSYM_NAME_LEN]),
            Some(("alias_do_fork", 0x100, 0, 'T'))
        );
        assert_eq!(
            mapped.lookup_address(0x1250, &mut [0; KSYM_NAME_LEN]),
            Some(("cpu_startup_entry", 0x100, 0x50, 'T'))
        );
        assert_eq!(
            mapped.lookup_address(0x1450, &mut [0; KSYM_NAME_LEN]),
            Some(("cpu_startup_entry", 0x100, 0x50, 'T'))
        );
        assert_eq!(
            mapped.lookup_name("cpu_startup_entry"),
            Some(0x1200),
            "The address of cpu_startup_entry should be 0x1200 instead of 0x1400"
        );
        assert_eq!(mapped.lookup_name("do_fork_2"), Some(0x1300));

        println!("All tests passed.");
        let dumped = mapped.dump_all_symbols();
        println!("Dumped all symbols:\n{}", dumped);
    }

    fn trans<'b>(buf: &'b mut [u8; 10]) -> &'b str {
        buf.copy_from_slice(b"abcdabcdab");
        std::str::from_utf8(buf).unwrap()
    }
    #[test]
    fn k() {
        let mut buf = [0u8; 10];
        assert_eq!(trans(&mut buf), "abcdabcdab");
    }
}
