// SPDX-License-Identifier: MPL-2.0

use crate::{
    scml_parser::{
        ParserCtx, Pattern, PatternArg, PatternArray, PatternFlagSet, PatternStruct, Patterns,
    },
    strace_parser::{Syscall, SyscallArg, SyscallArray, SyscallFlagSet, SyscallStruct},
};

/// Matcher for syscalls against SCML patterns.
pub(crate) struct Matcher<'a> {
    patterns: Patterns<'a>,
}

impl<'a> Matcher<'a> {
    pub(crate) fn new(patterns: Patterns<'a>) -> Self {
        Self { patterns }
    }

    /// Attempts to match a syscall against available patterns.
    pub(crate) fn match_syscall(&self, syscall: &Syscall) -> Option<&Pattern<'_>> {
        let related_patterns = self.patterns.get(syscall.name())?;

        related_patterns
            .iter()
            .find(|pattern| Self::matches(self.patterns.ctx(), syscall, pattern))
    }

    /// Matches a syscall against a specific pattern.
    fn matches(ctx: &ParserCtx, syscall: &Syscall, pattern: &Pattern) -> bool {
        assert_eq!(syscall.name(), pattern.name());

        // Argument count must match exactly if wildcard is not enabled
        if !pattern.wildcard() && syscall.args().len() != pattern.args().len() {
            return false;
        }

        // syscall arguments must be at least as many as pattern arguments
        if syscall.args().len() < pattern.args().len() {
            return false;
        }

        // Match each argument pair
        for (syscall_arg, pattern_arg) in syscall.args().iter().zip(pattern.args().iter()) {
            if !Self::matches_arg(ctx, syscall_arg, pattern_arg) {
                return false;
            }
        }

        true
    }

    /// Matches a single syscall argument against a pattern argument.
    fn matches_arg(ctx: &ParserCtx, syscall_arg: &SyscallArg, pattern_arg: &PatternArg) -> bool {
        match pattern_arg {
            // Matches any syscall argument
            PatternArg::None => true,

            // Type constraint - matches only integer arguments
            PatternArg::Integer => matches!(syscall_arg, SyscallArg::Integer(_)),

            // Path matching not yet implemented
            PatternArg::Path => todo!("Path matching not implemented yet"),

            // Flag set matching - supports both zero values and flag combinations
            PatternArg::Flags(pattern_flags) => match syscall_arg {
                // Special case: integer zero matches any flag pattern
                SyscallArg::Integer(value) if *value == "0" => true,
                // Validate that all syscall flags are present in pattern
                SyscallArg::Flags(syscall_flags) => {
                    Self::matches_flags(ctx, syscall_flags, pattern_flags)
                }
                _ => false,
            },

            // Array matching - recursive element validation
            PatternArg::Array(pattern_array) => match syscall_arg {
                SyscallArg::Array(syscall_array) => {
                    Self::matches_array(ctx, syscall_array, pattern_array)
                }
                // Handle NULL array case - single Flag("NULL") matches any array pattern
                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            // Struct matching - field-by-field validation with wildcard support
            PatternArg::Struct(pattern_struct) => match syscall_arg {
                SyscallArg::Struct(syscall_struct) => {
                    Self::matches_struct(ctx, syscall_struct, pattern_struct)
                }
                // Handle NULL pointer case - single Flag("NULL") matches any struct pattern
                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            // Multiple struct alternatives - matches if any alternative matches
            PatternArg::MultipleStruct(pattern_structs) => match syscall_arg {
                SyscallArg::Struct(syscall_struct) => {
                    pattern_structs.structs().iter().any(|pattern_struct| {
                        if let PatternArg::Struct(pattern_struct) = pattern_struct {
                            Self::matches_struct(ctx, syscall_struct, pattern_struct)
                        } else {
                            panic!("Expected PatternArg::Struct inside MultipleStruct");
                        }
                    })
                }
                // Handle NULL pointer case - single Flag("NULL") matches any struct pattern
                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            PatternArg::FlagsVariable(_) | PatternArg::StructVariable(_) => {
                let expanded = pattern_arg.get(ctx);
                Self::matches_arg(ctx, syscall_arg, expanded)
            }

            // Single flags should not appear in patterns
            PatternArg::Flag(_) => {
                panic!(
                    "Single Flag should not appear in pattern matching - should be wrapped in Flags"
                )
            }
        }
    }

    fn matches_flags(
        ctx: &ParserCtx,
        syscall_flags: &SyscallFlagSet,
        pattern_flags: &PatternFlagSet,
    ) -> bool {
        // Every syscall flag must find a match in the pattern flags
        syscall_flags
            .flags()
            .iter()
            .all(|syscall_flag| Self::matches_flag(ctx, syscall_flag, pattern_flags))
    }

    /// Matches a single syscall flag against a pattern flag set.
    fn matches_flag(
        ctx: &ParserCtx,
        syscall_flag: &SyscallArg,
        pattern_flags: &PatternFlagSet,
    ) -> bool {
        match syscall_flag {
            SyscallArg::Flag(_) => (),
            _ => {
                panic!("Syscall flag must be of type Flag for flag matching");
            }
        };

        let mut found_match = false;

        for pattern_flag in pattern_flags.flags() {
            match (syscall_flag, pattern_flag) {
                // Named flag matching - exact name comparison
                (SyscallArg::Flag(syscall_name), PatternArg::Flag(pattern_name)) => {
                    if syscall_name == pattern_name {
                        found_match = true;
                        break;
                    }
                }

                // Integer flag matching - pattern accepts any integer
                (SyscallArg::Integer(_), PatternArg::Integer) => {
                    found_match = true;
                    break;
                }

                // Flags variable - expand and attempt matching
                (_, PatternArg::FlagsVariable(_)) => {
                    if let PatternArg::Flags(expanded) = pattern_flag.get(ctx) {
                        found_match = Self::matches_flag(ctx, syscall_flag, expanded);
                        if found_match {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        found_match
    }

    fn matches_array(
        ctx: &ParserCtx,
        syscall_array: &SyscallArray,
        pattern_array: &PatternArray,
    ) -> bool {
        syscall_array.elements().iter().all(|syscall_element| {
            pattern_array
                .args()
                .iter()
                .any(|pattern_element| Self::matches_arg(ctx, syscall_element, pattern_element))
        })
    }

    fn matches_struct(
        ctx: &ParserCtx,
        syscall_struct: &SyscallStruct,
        pattern_struct: &PatternStruct,
    ) -> bool {
        // Without wildcard, field count must match exactly
        if !pattern_struct.wildcard()
            && syscall_struct.fields().len() != pattern_struct.fields().len()
        {
            return false;
        }

        // Every pattern field must be found and matched in the syscall struct
        for (pattern_field_name, pattern_field_value) in pattern_struct.fields() {
            match syscall_struct.get_value(pattern_field_name) {
                Some(syscall_field_value) => {
                    if !Self::matches_arg(ctx, syscall_field_value, pattern_field_value) {
                        return false;
                    }
                }
                None => {
                    // Required pattern field not found in syscall
                    return false;
                }
            }
        }

        true
    }
}
