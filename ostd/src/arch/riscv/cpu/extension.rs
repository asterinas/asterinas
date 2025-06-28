// SPDX-License-Identifier: MPL-2.0

//! RISC-V ISA extensions.

use bitflags::bitflags;
use spin::Once;

use crate::arch::boot::DEVICE_TREE;

/// Detects available RISC-V ISA extensions.
pub fn init() {
    let mut global_isa_extensions = IsaExtensions::all();

    let device_tree = DEVICE_TREE.get().expect("Device tree not initialized");
    let mut cpu_count = 0;

    for cpu in device_tree.cpus() {
        cpu_count += 1;

        let cpu_isa_extensions = if let Some(isa_extensions) = cpu.property("riscv,isa-extensions")
        {
            parse_isa_extensions_list(&isa_extensions)
        } else if let Some(isa) = cpu.property("riscv,isa") {
            parse_isa_string(&isa)
        } else {
            log::error!(
                "CPU {} has no riscv,isa or riscv,isa-extensions property",
                cpu_count - 1
            );
            continue;
        };

        global_isa_extensions &= cpu_isa_extensions;
    }

    GLOBAL_ISA_EXTENSIONS.call_once(|| global_isa_extensions);
}

/// Checks if the specified set of ISA extensions are available.
pub fn has_extensions(required: IsaExtensions) -> bool {
    GLOBAL_ISA_EXTENSIONS.get().unwrap().contains(required)
}

fn parse_isa_string(isa: &fdt::node::NodeProperty) -> IsaExtensions {
    let mut extensions = IsaExtensions::empty();
    let isa_str = isa.as_str().unwrap();

    let mut ext_iter = {
        let ext_start = if isa_str.starts_with("rv32") || isa_str.starts_with("rv64") {
            4
        } else {
            0
        };
        if ext_start >= isa_str.len() {
            return extensions;
        }
        isa_str[ext_start..].split('_')
    };

    // Parse single-letter extensions from first part
    if let Some(first_part) = ext_iter.next() {
        for ch in first_part.chars() {
            if let Some(ext_data) = EXTENSION_TABLE
                .iter()
                .find(|e| e.name.len() == 1 && e.name.chars().next() == Some(ch))
            {
                extensions |= ext_data.flag;
            }
        }
    }

    // Parse multi-letter extensions from remaining parts
    for part in ext_iter {
        if let Some(ext_data) = EXTENSION_TABLE.iter().find(|e| e.name == part) {
            extensions |= ext_data.flag;
        }
    }

    extensions
}

fn parse_isa_extensions_list(isa_extensions: &fdt::node::NodeProperty) -> IsaExtensions {
    let mut extensions = IsaExtensions::empty();
    let isa_extensions_list = isa_extensions.value;

    for str in isa_extensions_list.split(|&b| b == 0) {
        if str.is_empty() {
            continue;
        }
        if let Ok(ext_name) = core::str::from_utf8(str) {
            if let Some(ext_data) = EXTENSION_TABLE.iter().find(|e| e.name == ext_name) {
                extensions |= ext_data.flag;
            }
        }
    }

    extensions
}

static GLOBAL_ISA_EXTENSIONS: Once<IsaExtensions> = Once::new();

/// A macro for RISC-V ISA extension definition and lookup table generation.
macro_rules! define_isa_extensions {
    (
        $(
            $name:ident = $bit:expr, $str:expr, $doc:expr;
        )*
    ) => {
        bitflags! {
            /// RISC-V ISA extensions
            pub struct IsaExtensions: u128 {
                $(
                    #[doc = $doc]
                    const $name = 1u128 << $bit;
                )*
            }
        }

        const EXTENSION_TABLE: &[ExtensionData] = &[
            $(
                ExtensionData {
                    name: $str,
                    flag: IsaExtensions::$name
                },
            )*
        ];
    };
}

struct ExtensionData {
    name: &'static str,
    flag: IsaExtensions,
}

define_isa_extensions! {
    // Standard single-letter extensions (0-25)
    A =  0, "a", "Atomic instructions";
    C =  2, "c", "Compressed instructions";
    D =  3, "d", "Double-precision floating-point";
    F =  5, "f", "Single-precision floating-point";
    H =  7, "h", "Hypervisor";
    I =  8, "i", "Base integer instruction set";
    M = 12, "m", "Integer multiplication and division";
    Q = 16, "q", "Quad-precision floating-point";
    V = 21, "v", "Vector extension";

    // Multi-letter extensions
    SSCOFPMF    = 26, "sscofpmf",    "Supervisor-mode counter overflow and privilege mode filtering";
    SSTC        = 27, "sstc",        "Supervisor-mode timer interrupts";
    SVINVAL     = 28, "svinval",     "Fine-grained address-translation cache invalidation";
    SVPBMT      = 29, "svpbmt",      "Page-based memory types";
    ZBB         = 30, "zbb",         "Basic bit manipulation";
    ZICBOM      = 31, "zicbom",      "Cache block management operations";
    ZIHINTPAUSE = 32, "zihintpause", "Pause hint";
    SVNAPOT     = 33, "svnapot",     "NAPOT translation contiguity";
    ZICBOZ      = 34, "zicboz",      "Cache block zero operations";
    SMAIA       = 35, "smaia",       "Advanced interrupt architecture (machine mode)";
    SSAIA       = 36, "ssaia",       "Advanced interrupt architecture (supervisor mode)";
    ZBA         = 37, "zba",         "Address generation for bit manipulation";
    ZBS         = 38, "zbs",         "Single-bit instructions";
    ZICNTR      = 39, "zicntr",      "Base counters and timers";
    ZICSR       = 40, "zicsr",       "Control and status register instructions";
    ZIFENCEI    = 41, "zifencei",    "Instruction-fetch fence";
    ZIHPM       = 42, "zihpm",       "Hardware performance counters";
    SMSTATEEN   = 43, "smstateen",   "State enable extension";
    ZICOND      = 44, "zicond",      "Integer conditional operations";
    ZBC         = 45, "zbc",         "Carry-less multiplication";
    ZBKB        = 46, "zbkb",        "Bit manipulation instructions for cryptography";
    ZBKC        = 47, "zbkc",        "Carry-less multiplication for cryptography";
    ZBKX        = 48, "zbkx",        "Crossbar permutation instructions";
    ZKND        = 49, "zknd",        "AES decryption";
    ZKNE        = 50, "zkne",        "AES encryption";
    ZKNH        = 51, "zknh",        "Hash function instructions";
    ZKR         = 52, "zkr",         "Entropy source";
    ZKSED       = 53, "zksed",       "SM4 encryption/decryption";
    ZKSH        = 54, "zksh",        "SM3 hash function";
    ZKT         = 55, "zkt",         "Data-independent execution latency";
    ZVBB        = 56, "zvbb",        "Vector basic bit manipulation";
    ZVBC        = 57, "zvbc",        "Vector carry-less multiplication";
    ZVKB        = 58, "zvkb",        "Vector bit manipulation instructions for cryptography";
    ZVKG        = 59, "zvkg",        "Vector GCM/GMAC";
    ZVKNED      = 60, "zvkned",      "Vector AES encryption/decryption";
    ZVKNHA      = 61, "zvknha",      "Vector SHA-2 (SHA-256 and SHA-224)";
    ZVKNHB      = 62, "zvknhb",      "Vector SHA-2 (SHA-512, SHA-384, SHA-256, SHA-224)";
    ZVKSED      = 63, "zvksed",      "Vector SM4 encryption/decryption";
    ZVKSH       = 64, "zvksh",       "Vector SM3 hash function";
    ZVKT        = 65, "zvkt",        "Vector data-independent execution latency";
    ZFH         = 66, "zfh",         "Half-precision floating-point";
    ZFHMIN      = 67, "zfhmin",      "Minimal half-precision floating-point";
    ZIHINTNTL   = 68, "zihintntl",   "Non-temporal locality hints";
    ZVFH        = 69, "zvfh",        "Vector half-precision floating-point";
    ZVFHMIN     = 70, "zvfhmin",     "Vector minimal half-precision floating-point";
    ZFA         = 71, "zfa",         "Additional floating-point instructions";
    ZTSO        = 72, "ztso",        "Total store ordering";
    ZACAS       = 73, "zacas",       "Atomic compare-and-swap";
    ZVE32X      = 74, "zve32x",      "Vector extension for embedded processors (32-bit)";
    ZVE32F      = 75, "zve32f",      "Vector extension for embedded processors (32-bit with float)";
    ZVE64X      = 76, "zve64x",      "Vector extension for embedded processors (64-bit)";
    ZVE64F      = 77, "zve64f",      "Vector extension for embedded processors (64-bit with float)";
    ZVE64D      = 78, "zve64d",      "Vector extension for embedded processors (64-bit with double)";
    ZIMOP       = 79, "zimop",       "May-be-operations";
    ZCA         = 80, "zca",         "Compressed instructions (A subset)";
    ZCB         = 81, "zcb",         "Compressed instructions (B subset)";
    ZCD         = 82, "zcd",         "Compressed instructions (D subset)";
    ZCF         = 83, "zcf",         "Compressed instructions (F subset)";
    ZCMOP       = 84, "zcmop",       "Compressed may-be-operations";
    ZAWRS       = 85, "zawrs",       "Wait-for-reservation-set instructions";
    SVVPTC      = 86, "svvptc",      "Vectored page table cache";
    SMMPM       = 87, "smmpm",       "Machine-mode pointer masking";
    SMNPM       = 88, "smnpm",       "Machine-mode pointer masking (non-pointer)";
    SSNPM       = 89, "ssnpm",       "Supervisor-mode pointer masking (non-pointer)";
    ZABHA       = 90, "zabha",       "Byte and halfword atomic memory operations";
    ZICCRSE     = 91, "ziccrse",     "Main memory supports forward progress on LR/SC sequences";
    SVADE       = 92, "svade",       "Hardware A/D bit updates";
    SVADU       = 93, "svadu",       "Hardware A/D bit updates (user mode)";
    ZFBFMIN     = 94, "zfbfmin",     "Scalar BF16 converts";
    ZVFBFMIN    = 95, "zvfbfmin",    "Vector BF16 converts";
    ZVFBFWMA    = 96, "zvfbfwma",    "Vector BF16 widening mul-add";
    ZAAMO       = 97, "zaamo",       "Atomic memory operations";
    ZALRSC      = 98, "zalrsc",      "Load-reserved/store-conditional";
}

#[cfg(ktest)]
mod tests {
    use super::*;
    use crate::prelude::*;

    struct MockFdtProperty {
        data: Vec<u8>,
    }

    impl MockFdtProperty {
        fn new_string(string: &str) -> Self {
            let mut data = string.as_bytes().to_vec();
            data.push(0);
            Self { data }
        }

        fn new_string_list(strings: &[&str]) -> Self {
            let mut data = Vec::new();
            for string in strings {
                data.extend_from_slice(string.as_bytes());
                data.push(0);
            }
            Self { data }
        }

        // For the extensions list parser
        fn value(&self) -> &[u8] {
            &self.data
        }
    }

    fn parse_isa_string_wrapper(string: &str) -> IsaExtensions {
        let prop = MockFdtProperty::new_string(string);
        let node = fdt::node::NodeProperty {
            name: "riscv,isa",
            value: prop.value(),
        };
        parse_isa_string(&node)
    }

    fn parse_isa_extensions_list_wrapper(strings: &[&str]) -> IsaExtensions {
        let prop = MockFdtProperty::new_string_list(strings);
        let node = fdt::node::NodeProperty {
            name: "riscv,isa-extensions",
            value: prop.value(),
        };
        parse_isa_extensions_list(&node)
    }

    #[ktest]
    fn isa_string_with_basic() {
        let result = parse_isa_string_wrapper("rv64imafdc_zicsr_zifencei");
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));
        assert!(result.contains(IsaExtensions::A));
        assert!(result.contains(IsaExtensions::F));
        assert!(result.contains(IsaExtensions::D));
        assert!(result.contains(IsaExtensions::C));
        assert!(result.contains(IsaExtensions::ZICSR));
        assert!(result.contains(IsaExtensions::ZIFENCEI));
        assert!(!result.contains(IsaExtensions::V));
        assert!(!result.contains(IsaExtensions::H));
    }

    #[ktest]
    fn isa_string_edge_cases() {
        // Empty string
        let result = parse_isa_string_wrapper("");
        assert!(result.is_empty());

        // Empty after prefix
        let result = parse_isa_string_wrapper("rv64");
        assert!(result.is_empty());

        // No prefix
        let result = parse_isa_string_wrapper("imafdc");
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));
        assert!(result.contains(IsaExtensions::A));

        // Only multi-letter extensions
        let result = parse_isa_string_wrapper("rv64_zicsr_zifencei");
        assert!(result.contains(IsaExtensions::ZICSR));
        assert!(result.contains(IsaExtensions::ZIFENCEI));
        assert!(!result.contains(IsaExtensions::I));
    }

    #[ktest]
    fn isa_string_unknown_extensions() {
        // Should ignore unknown extensions without crashing
        let result = parse_isa_string_wrapper("rv64imafdc_zunknown_zicsr_zifencei");
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));
        assert!(result.contains(IsaExtensions::A));
        assert!(result.contains(IsaExtensions::F));
        assert!(result.contains(IsaExtensions::D));
        assert!(result.contains(IsaExtensions::C));
        assert!(result.contains(IsaExtensions::ZICSR));
        assert!(result.contains(IsaExtensions::ZIFENCEI));
    }

    #[ktest]
    fn isa_extensions_list_basic() {
        let result =
            parse_isa_extensions_list_wrapper(&["i", "m", "a", "f", "d", "c", "zicsr", "zifencei"]);
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));
        assert!(result.contains(IsaExtensions::A));
        assert!(result.contains(IsaExtensions::F));
        assert!(result.contains(IsaExtensions::D));
        assert!(result.contains(IsaExtensions::C));
        assert!(result.contains(IsaExtensions::ZICSR));
        assert!(result.contains(IsaExtensions::ZIFENCEI));
        assert!(!result.contains(IsaExtensions::V));
        assert!(!result.contains(IsaExtensions::H));
    }

    #[ktest]
    fn isa_extensions_list_edge_cases() {
        // Empty list
        let result = parse_isa_extensions_list_wrapper(&[]);
        assert!(result.is_empty());

        // List with empty strings
        let result = parse_isa_extensions_list_wrapper(&["", "i", "", "m", ""]);
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));

        // Only empty strings
        let result = parse_isa_extensions_list_wrapper(&["", "", ""]);
        assert!(result.is_empty());
    }

    #[ktest]
    fn isa_extensions_list_unknown_extensions() {
        // Should ignore unknown extensions without crashing
        let result = parse_isa_extensions_list_wrapper(&[
            "i", "m", "a", "f", "d", "c", "zunknown", "zicsr", "zifencei",
        ]);
        assert!(result.contains(IsaExtensions::I));
        assert!(result.contains(IsaExtensions::M));
        assert!(result.contains(IsaExtensions::A));
        assert!(result.contains(IsaExtensions::F));
        assert!(result.contains(IsaExtensions::D));
        assert!(result.contains(IsaExtensions::C));
        assert!(result.contains(IsaExtensions::ZICSR));
        assert!(result.contains(IsaExtensions::ZIFENCEI));
    }
}
