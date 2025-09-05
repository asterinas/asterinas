// SPDX-License-Identifier: MPL-2.0

//! x86 ISA extensions.

use core::arch::x86_64::CpuidResult;

use bitflags::bitflags;
use spin::Once;

use super::cpuid::cpuid;

/// Detects available x86 ISA extensions.
pub(in crate::arch) fn init() {
    let mut global_isa_extensions = IsaExtensions::empty();

    for ext_leaf in EXTENSION_TABLE.iter() {
        let Some(CpuidResult { ebx, ecx, edx, .. }) = cpuid(ext_leaf.leaf, ext_leaf.subleaf) else {
            continue;
        };

        for ext_data in ext_leaf.data.iter() {
            let bits = match ext_data.reg {
                Reg::Ebx => ebx,
                Reg::Ecx => ecx,
                Reg::Edx => edx,
            };
            if bits & (1 << ext_data.bit) != 0 {
                global_isa_extensions |= ext_data.flag;
            }
        }
    }

    log::info!("Detected ISA extensions: {:?}", global_isa_extensions);

    GLOBAL_ISA_EXTENSIONS.call_once(|| global_isa_extensions);
}

/// Checks if the specified set of ISA extensions are available.
pub fn has_extensions(required: IsaExtensions) -> bool {
    GLOBAL_ISA_EXTENSIONS.get().unwrap().contains(required)
}

static GLOBAL_ISA_EXTENSIONS: Once<IsaExtensions> = Once::new();

macro_rules! define_isa_extensions {
    { $(leaf $leaf:literal, subleaf $subleaf:literal => {
        $($name:ident, $reg:ident ($bit:literal), $doc:literal; )*
    })* } => {
        define_isa_extension_type! {
            $($($name, $doc;)*)*
        }

        const EXTENSION_TABLE: &[ExtensionLeaf] = &[
            $(ExtensionLeaf {
                leaf: $leaf,
                subleaf: $subleaf,
                data: &[
                    $(ExtensionData {
                        reg: Reg::$reg,
                        bit: $bit,
                        flag: IsaExtensions::$name,
                    },)*
                ]
            },)*
        ];
    };
}

macro_rules! define_isa_extension_type {
    { $($name:ident, $doc:literal;)* } => {
        bitflags! {
            /// x86 ISA extensions.
            pub struct IsaExtensions: u128 {
                $(
                    #[doc = $doc]
                    const $name = 1u128 << ${index()};
                )*
            }
        }
    };
}

/// Extensions that describe in a CPUID leaf.
struct ExtensionLeaf {
    leaf: u32,
    subleaf: u32,
    data: &'static [ExtensionData],
}

/// An extension and its position (i.e., the register and the bit) in the CPUID result.
struct ExtensionData {
    reg: Reg,
    bit: u32,
    flag: IsaExtensions,
}

enum Reg {
    Ebx,
    Ecx,
    Edx,
}

define_isa_extensions! {
    leaf 1, subleaf 0 => {
        X2APIC,       Ecx(21), "The processor supports x2APIC feature.";
        TSC_DEADLINE, Ecx(24), "The processor's local APIC timer supports \
                                one-shot operation using a TSC deadline value.";
        XSAVE,        Ecx(26), "The processor supports the XSAVE/XRSTOR \
                                processor extended states feature, \
                                the XSETBV/XGETBV instructions, and XCR0.";
        AVX,          Ecx(28), "The processor supports the AVX instruction extensions.";
        RDRAND,       Ecx(30), "The processor supports RDRAND instruction.";

        XAPIC,        Edx( 9), "APIC On-Chip.";
    }

    leaf 7, subleaf 0 => {
        FSGSBASE,     Ebx( 0), "Supports RDFSBASE/RDGSBASE/WRFSBASE/WRGSBASE.";
        AVX512F,      Ebx(16), "Supports the AVX512F instruction extensions.";
    }
}
