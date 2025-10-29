# ksym-bin Guide

This directory provides a tool that reads kallsyms-like symbol lists (e.g., from /proc/kallsyms) from stdin and produces a compressed binary blob, along with a zero-copy reading convention for consumers.

This document covers:
- Relationships among KallsymsBlob metadata
- Compression and token selection rules
- Symbol ordering and lookup
- Binary layout and alignment (for zero-copy)
- Limitations and notes

## KallsymsBlob metadata

KallsymsBlob is a build-time container used to collect symbols, compress them, and serialize the result. Key fields:

- token_table: Vec<u8>
  - Concatenated bytes of all token strings.
- token_index: Vec<u32>
  - Start byte offsets for each token in token_table; indices match token ids (0..N-1).
- token_map: HashMap<String, u16>
  - Build-time dictionary (not serialized) mapping token text to its id.
- kallsyms_names: Vec<u8>
  - Concatenated compressed bytes of all symbol names.
- kallsyms_offsets: Vec<u32>
  - For each address-ordered symbol, the start byte offset into `kallsyms_names`.
- kallsyms_seqs_of_names: Vec<u32>
  - Mapping from name-order to address-order indices (used for binary search by name).
- kallsyms_addresses: Vec<u64>
  - Symbol virtual addresses in ascending order (address order).
- kallsyms_num_syms: usize
  - Number of symbols (also written as a u64 header in the blob).

Relationship:
- For the i-th symbol in address order:
  - Address = `kallsyms_addresses[i]`
  - Name record begins at `kallsyms_offsets[i]`. Each name record starts with a 1-byte type prefix `TY_LEN=1` and a 2-byte length prefix `LENGTH_BYTES=2` (little-endian), followed by `entry_len` bytes of payload. The actual slice is:
    `kallsyms_names[kallsyms_offsets[i] + PREFIX_LEN .. kallsyms_offsets[i] + PREFIX_LEN + entry_len] (PREFIX_LEN = TY_LEN + LENGTH_BYTES)`.
- Name search: binary search over `kallsyms_seqs_of_names`; once `mid` is found, retrieve the address-order index `seq = kallsyms_seqs_of_names[mid]`.

## Compression and token selection

- Token candidates are collected via a fixed-prefix heuristic: only prefixes of each name are considered. The length set is implementation-configured (currently several discrete lengths such as 10, 24, 31, …, 2000).
  - If a name is shorter than the smallest length (currently 10), the whole name is also counted as a candidate (for position 0 only).
  - Candidates are ranked by a weighted score (frequency × length), and up to 512 tokens are selected. Tokens that are prefixes of already-selected tokens are skipped to reduce redundancy.
- Name compression uses “single-prefix token + raw remainder”:
  - Try only one token at the beginning of the name (longer candidates first);
  - If matched, emit the token code and then append the remaining raw bytes; otherwise, write the whole name as raw bytes.
- Each name record is encoded as a “fixed-size prefix + payload”:
  - A 1-byte type prefix;
  - A 2-byte little-endian length prefix specifies the size of the payload;
  - If a token is used, the payload starts with `0xFF <id> 0xFF` (1-byte id) or `0xFF <id_hi> <id_lo> 0xFF` (2-byte id), followed by the remaining raw bytes;
  - If no token is used, the payload is simply the raw name bytes.
- Token marker constant: `TOKEN_MARKER = 0xFF`.

Note: Only text symbols (T/t) are kept from the input, and Rust demangling is applied. During encoding, the symbol type character (T/t) is prefixed to the compressed name.

## Symbol ordering and lookup

- Storage order is by address. During build:
  - `kallsyms_addresses` and `kallsyms_offsets` are sorted ascending by address;
  - A name-order → address-order mapping `kallsyms_seqs_of_names` is built for name binary search.
- Lookup:
  - Address → symbol: binary-search `kallsyms_addresses` to get index i. To handle aliasing (multiple symbols at the same address), search backward to the first symbol at that address, then search forward to the next symbol with a different address to determine the size. If none is found, use the end of the text section as the upper bound. Name decoding uses the length prefix at `kallsyms_offsets[i]` to slice the record, then expand.
  - Name → address: binary-search in name space. Each step, use `kallsyms_seqs_of_names[mid]` to fetch the address-order index, decode the name record using the length prefix, and compare. On match, return the corresponding address.

## Binary layout and alignment

All fields are serialized in little-endian. To support zero-copy reading with proper alignment (u64 aligned to 8 bytes, u32 aligned to 4 bytes), padding is inserted before each segment as needed. The loader maps the blob at a 4K-aligned address. With this guarantee, zero-copy casting via `from_raw_parts` is safe.

Segment order (alignment in parentheses):

1) num_syms: u64
2) addresses[num_syms]: u64[]  (align 8)
3) offsets[num_syms]: u32[]    (align 4)
4) seqs[num_syms]: u32[]       (align 4)
5) names:                      (align 8)
   - names_len: u64
   - names_bytes[names_len]: u8[]
     - repeated name records: `[[type: u8] [len: u16(le)] [payload: u8[len]]`
6) token_table:                (align 8)
   - token_table_len: u64
   - token_table_bytes[token_table_len]: u8[]
7) token_index:                (align 8 for len, then align 4 for array)
   - token_index_len: u64
   - token_index[token_index_len]: u32[] (align 4)

Notes:
- Alignment is ensured by padding the output buffer before each segment.
- `kallsyms_offsets` uses u32; thus `kallsyms_names.len()` must be < 4 GiB.

## Zero-copy reading (KallsymsMapped)

- `from_blob(&blob, stext, etext)` interprets the segments directly as slices while remembering the text section bounds for address lookup:
  - `&[u64]` for addresses,
  - `&[u32]` for offsets, seqs, and token_index,
  - `&[u8]`  for names and token_table.
- Because each segment begins at an 8/4-byte boundary and the overall mapping is 4K-aligned, `from_raw_parts` alignment is satisfied.

## Limitations and notes

- Up to 512 tokens; token id is u16.
- Each name record has a 2-byte little-endian length; maximum payload per record is 65535 bytes.
- `kallsyms_offsets` is u32, limiting the total size of `kallsyms_names` to < 4 GiB.
- All top-level integers (num_syms, addresses, offsets, seqs, names_len, token_table_len, token_index_len, and token_index contents) are little-endian; name record length is also little-endian.
- Compression applies at most one token at the beginning; the rest remains raw bytes. Heavier compression strategies can be added in the future if needed.

## Usage

- Generation:
  - Pipe `nm -n -C {ELF}` (keeping T/t only) into the generator, e.g.:
    - `nm -n -C {ELF} | cargo run -p ksym-bin --bin gen_ksym --features demangle > kallsyms.bin`
- Reading (consumer):
  - Use `ksym_bin::KallsymsMapped::from_blob(&blob, stext, etext)` for zero-copy parsing;
  - Use `lookup_address`, `lookup_name`, or `dump_all_symbols()` (dump line format: `<addr_hex> <type_char> <name>`).

## Tests

```
cargo test --bin gen_ksym --features="demangle"
```

## Example

Example with three simplified symbols:
- Input:
  - 0000000000001000 T _start
  - 0000000000001100 T do_fork
  - 0000000000001200 T cpu_startup_entry
- In the resulting blob:
  - addresses = [0x1000, 0x1100, 0x1200]
  - offsets point to the beginning of each compressed name record in `kallsyms_names`;
  - seqs reflect the mapping from name order to address order;
  - names/token_table/token_index are serialized with the alignment described above.

That summarizes the metadata relationships and binary layout for the current implementation.
