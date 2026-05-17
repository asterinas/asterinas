// SPDX-License-Identifier: MPL-2.0

//! CPU extension/feature detection.

/// ISA extensions for ARM64.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsaExtensions {
    /// FP (floating point) extension.
    FP,
    /// SIMD (Advanced SIMD/NEON) extension.
    SIMD,
    /// SVE extension.
    SVE,
}

/// Detects if the given extensions are available.
pub fn has_extensions(extension: IsaExtensions) -> bool {
    match extension {
        IsaExtensions::FP => true,
        IsaExtensions::SIMD => true,
        IsaExtensions::SVE => false,
    }
}

/// Initializes CPU extensions on the current CPU.
pub fn init() {
    // TODO: Parse CPU feature ID registers (ID_AA64ISAR0_EL1, etc.)
}
