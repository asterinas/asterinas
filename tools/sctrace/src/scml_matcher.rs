// SPDX-License-Identifier: MPL-2.0

//! SCML pattern matcher for syscall validation.
//!
//! This module provides functionality to match parsed syscalls from strace output
//! against SCML pattern definitions. The matcher supports:
//!
//! - Type-based matching (integers, paths, flags, structs, arrays)
//! - Flag subset validation (syscall flags must be subset of pattern flags)
//! - Recursive matching for nested structures
//! - Wildcard support for flexible argument and field matching
//! - NULL pointer handling (treats NULL as matching any pointer type)
//! - Variable expansion for flags and struct definitions
//!
//! # Example
//!
//! ```text
//! use scml_matcher::Matcher;
//! use scml_parser::Patterns;
//! use strace_parser::Syscall;
//!
//! // Load patterns from SCML file
//! let patterns = Patterns::from_scml_file("patterns.scml")?;
//! let matcher = Matcher::new(patterns);
//!
//! // Parse syscall from strace output
//! let syscall = Syscall::parse("openat(AT_FDCWD, \"file.txt\", O_RDONLY) = 3")?;
//!
//! // Match syscall against patterns
//! if let Some(pattern) = matcher.match_syscall(&syscall) {
//!     println!("Matched pattern: {}", pattern.name());
//! }
//! ```

use crate::{
    scml_parser::{ParserCtx, Pattern, Patterns},
    strace_parser::Syscall,
};

/// Matcher for syscalls against SCML patterns.
///
/// This struct provides functionality to match parsed syscalls from strace output
/// against pattern definitions loaded from SCML files. It maintains a collection
/// of patterns organized by syscall name for efficient lookup and provides access
/// to the parser context for variable expansion.
///
/// # Matching Behavior
///
/// The matcher uses a first-match strategy: when multiple patterns exist for a
/// syscall, the first pattern that successfully matches is returned. Pattern
/// order is determined by the order they appear in the SCML file.
///
/// # Variable Expansion
///
/// The matcher automatically expands flag and struct variables during matching
/// using the parser context. This allows patterns to reference reusable definitions.
pub struct Matcher<'a> {
    /// Collection of SCML patterns organized by syscall name.
    patterns: Patterns<'a>,
}

impl<'a> Matcher<'a> {
    /// Creates a new Matcher with the given patterns.
    ///
    /// # Arguments
    ///
    /// * `patterns` - Collection of SCML patterns organized by syscall name,
    ///                including the parser context for variable expansion
    ///
    /// # Example
    ///
    /// ```text
    /// let patterns = Patterns::from_scml_file("patterns.scml")?;
    /// let matcher = Matcher::new(patterns);
    /// ```
    pub fn new(patterns: Patterns<'a>) -> Self {
        Self { patterns }
    }

    /// Attempts to match a syscall against available patterns.
    ///
    /// This method looks up patterns by syscall name and attempts to match
    /// the syscall against each pattern in order. Returns the first matching
    /// pattern, or `None` if no patterns match or no patterns exist for the
    /// syscall. Variable expansion is performed automatically during matching.
    ///
    /// # Arguments
    ///
    /// * `syscall` - The parsed syscall to match against patterns
    ///
    /// # Returns
    ///
    /// * `Some(&Pattern)` - Reference to the first matching pattern
    /// * `None` - No pattern matched the syscall, or no patterns exist for this syscall name
    ///
    /// # Example
    ///
    /// ```text
    /// let syscall = Syscall::parse("openat(AT_FDCWD, \"file.txt\", O_RDONLY) = 3")?;
    ///
    /// match matcher.match_syscall(&syscall) {
    ///     Some(pattern) => println!("Matched: {}", pattern.name()),
    ///     None => println!("No matching pattern found"),
    /// }
    /// ```
    pub fn match_syscall(&self, syscall: &Syscall) -> Option<&Pattern> {
        let related_patterns = self.patterns.get(syscall.name());

        let related_patterns = match related_patterns {
            Some(p) => p,
            None => return None,
        };

        for pattern in related_patterns {
            if Self::matches(self.patterns.ctx(), &syscall, &pattern) {
                return Some(&pattern);
            }
        }

        None
    }

    /// Matches a syscall against a specific pattern.
    ///
    /// This is the core matching logic that validates whether a syscall
    /// conforms to a pattern definition. The matching is performed in two steps:
    ///
    /// 1. **Argument Count Validation**: If the pattern doesn't have wildcard enabled,
    ///    the syscall must have exactly the same number of arguments as the pattern.
    ///    With wildcard enabled, the syscall may have additional arguments.
    ///
    /// 2. **Argument-by-Argument Matching**: Each argument is matched according to
    ///    its pattern type (see [`matches_arg`](Self::matches_arg) for detailed matching rules).
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context containing variable definitions for expansion
    /// * `syscall` - The syscall to match
    /// * `pattern` - The pattern to match against
    ///
    /// # Returns
    ///
    /// `true` if the syscall matches the pattern, `false` otherwise
    ///
    /// # Panics
    ///
    /// Panics if the syscall name doesn't match the pattern name (debug assertion).
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
    ///
    /// This method implements type-specific matching rules for each pattern type.
    /// Variables (`FlagsVariable` and `StructVariable`) are automatically expanded
    /// using the parser context before matching.
    ///
    /// # NULL Pointer Handling
    ///
    /// For pointer types (arrays and structs), a special case is handled:
    /// if the syscall argument is a single-element flag set containing `Flag("NULL")`,
    /// it matches any array or struct pattern. This handles NULL pointers in syscalls.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context for variable expansion
    /// * `syscall_arg` - The syscall argument to match
    /// * `pattern_arg` - The pattern argument to match against
    ///
    /// # Returns
    ///
    /// `true` if the arguments match according to the pattern type
    ///
    /// # Panics
    ///
    /// Panics if pattern contains a bare `Flag(_)` (should be wrapped in `Flags`)
    fn matches_arg(
        ctx: &ParserCtx,
        syscall_arg: &crate::strace_parser::SyscallArg,
        pattern_arg: &crate::scml_parser::PatternArg,
    ) -> bool {
        use crate::{scml_parser::PatternArg, strace_parser::SyscallArg};

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
                    match flag_set.flags().get(0) {
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL" => true,
                        _ => false,
                    }
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
                    match flag_set.flags().get(0) {
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL" => true,
                        _ => false,
                    }
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
                    match flag_set.flags().get(0) {
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL" => true,
                        _ => false,
                    }
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

    /// Matches a single syscall flag against a pattern flag set.
    ///
    /// This helper method checks if an individual syscall flag matches any flag
    /// in the pattern flag set. It supports:
    ///
    /// - Named flag matching by exact name comparison
    /// - Integer flag matching when pattern accepts `Integer` type
    /// - Automatic expansion of `FlagsVariable` in patterns
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context for variable expansion
    /// * `syscall_flag` - The syscall flag to match (must be `SyscallArg::Flag`)
    /// * `pattern_flags` - The pattern flag set to match against
    ///
    /// # Returns
    ///
    /// `true` if the syscall flag matches any flag in the pattern flag set
    ///
    /// # Panics
    ///
    /// Panics if `syscall_flag` is not of type `SyscallArg::Flag`
    ///
    /// # Example
    ///
    /// ```text
    /// Syscall flag:  O_RDONLY
    /// Pattern flags: [O_RDONLY, O_WRONLY, O_RDWR]
    /// Result:        true (exact match found)
    ///
    /// Syscall flag:  123
    /// Pattern flags: [<INTEGER>, O_CREAT]
    /// Result:        true (matches INTEGER pattern)
    /// ```
    fn matches_flag(
        ctx: &ParserCtx,
        syscall_flag: &crate::strace_parser::SyscallArg,
        pattern_flags: &crate::scml_parser::PatternFlagSet,
    ) -> bool {
        use crate::{scml_parser::PatternArg, strace_parser::SyscallArg};

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
                (_, PatternArg::FlagsVariable(_)) => {
                    let expanded_flags = pattern_flag.get(ctx);
                    if let PatternArg::Flags(expanded_flags) = expanded_flags {
                        if Self::matches_flag(ctx, syscall_flag, expanded_flags) {
                            found_match = true;
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        found_match
    }

    /// Matches syscall flags against pattern flags.
    ///
    /// Validates that every flag in the syscall flag set has a corresponding
    /// match in the pattern flag set. This implements **subset matching**:
    /// all syscall flags must be present in the pattern, but the pattern
    /// may contain additional flags. Variables in patterns are expanded
    /// automatically during matching.
    ///
    /// # Matching Rules
    ///
    /// - Named flags match by exact name comparison
    /// - Integer flags in syscall match pattern's `Integer` type
    /// - All syscall flags must find a match (AND logic)
    /// - Pattern may contain additional unmatched flags
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context for variable expansion
    /// * `syscall_flags` - The flag set from the syscall
    /// * `pattern_flags` - The flag set pattern to match against
    ///
    /// # Returns
    ///
    /// `true` if all syscall flags are matched by pattern flags
    ///
    /// # Example
    ///
    /// ```text
    /// Syscall:  O_RDWR | O_CREAT
    /// Pattern:  O_RDWR | O_CREAT | O_EXCL
    /// Result:   true (syscall flags are subset of pattern)
    ///
    /// Syscall:  O_RDWR | O_TRUNC
    /// Pattern:  O_RDWR | O_CREAT
    /// Result:   false (O_TRUNC not in pattern)
    /// ```
    fn matches_flags(
        ctx: &ParserCtx,
        syscall_flags: &crate::strace_parser::SyscallFlagSet,
        pattern_flags: &crate::scml_parser::PatternFlagSet,
    ) -> bool {
        // Every syscall flag must find a match in the pattern flags
        for syscall_flag in syscall_flags.flags() {
            if !Self::matches_flag(ctx, syscall_flag, pattern_flags) {
                return false;
            }
        }

        true
    }

    /// Matches syscall array against pattern array.
    ///
    /// Pattern arrays serve as templates: each element in the pattern array
    /// represents a valid element type. Every element in the syscall array
    /// must match **at least one** template element in the pattern array.
    ///
    /// This allows flexible array matching where the pattern defines what
    /// types of elements are acceptable, and the syscall array can contain
    /// any combination of these types.
    ///
    /// # Matching Strategy
    ///
    /// For each syscall array element:
    /// - Try matching against each pattern array element (OR logic)
    /// - If any pattern element matches, the syscall element is valid
    /// - All syscall elements must find at least one match (AND logic)
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context for variable expansion
    /// * `syscall_array` - The array from the syscall
    /// * `pattern_array` - The array pattern (contains template elements)
    ///
    /// # Returns
    ///
    /// `true` if all syscall array elements match at least one pattern element
    ///
    /// # Example
    ///
    /// ```text
    /// Syscall:  [123, 456, 789]
    /// Pattern:  [<INTEGER>]
    /// Result:   true (all elements are integers)
    ///
    /// Syscall:  [SA_NOCLDSTOP, SA_RESTART]
    /// Pattern:  [<SIGACTION_FLAGS>]  // expands to [SA_NOCLDSTOP, SA_RESTART, ...]
    /// Result:   true (all flags in pattern)
    /// ```
    fn matches_array(
        ctx: &ParserCtx,
        syscall_array: &crate::strace_parser::SyscallArray,
        pattern_array: &crate::scml_parser::PatternArray,
    ) -> bool {
        syscall_array.elements().iter().all(|syscall_element| {
            pattern_array
                .args()
                .iter()
                .any(|pattern_element| Self::matches_arg(ctx, syscall_element, pattern_element))
        })
    }

    /// Matches syscall struct against pattern struct.
    ///
    /// Validates struct field matching with support for wildcard fields.
    /// The matching behavior depends on the pattern's wildcard setting.
    /// Variable references in struct fields are expanded automatically.
    ///
    /// # Matching Rules
    ///
    /// ## Without Wildcard (`..` not specified)
    /// - Field count must match exactly
    /// - All pattern fields must exist in syscall with matching values
    /// - No additional fields allowed in syscall
    ///
    /// ## With Wildcard (`..` specified)
    /// - All pattern fields must exist in syscall with matching values
    /// - Syscall may contain additional fields beyond those in pattern
    /// - Field count validation is skipped
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context for variable expansion
    /// * `syscall_struct` - The struct from the syscall
    /// * `pattern_struct` - The struct pattern with field constraints
    ///
    /// # Returns
    ///
    /// `true` if all pattern fields are found and matched in the syscall struct
    ///
    /// # Example
    ///
    /// ```text
    /// // Exact match (no wildcard)
    /// Syscall:  {sa_flags=SA_NOCLDSTOP, sa_mask=[]}
    /// Pattern:  {sa_flags=SA_NOCLDSTOP, sa_mask=[]}
    /// Result:   true
    ///
    /// Syscall:  {sa_flags=SA_NOCLDSTOP, sa_mask=[], extra=123}
    /// Pattern:  {sa_flags=SA_NOCLDSTOP, sa_mask=[]}
    /// Result:   false (field count mismatch, no wildcard)
    ///
    /// // Wildcard match
    /// Syscall:  {sa_flags=SA_NOCLDSTOP, sa_mask=[], extra=123}
    /// Pattern:  {sa_flags=SA_NOCLDSTOP, ..}
    /// Result:   true (wildcard allows extra fields)
    ///
    /// Syscall:  {sa_flags=SA_NOCLDWAIT}
    /// Pattern:  {sa_flags=SA_NOCLDSTOP, ..}
    /// Result:   false (sa_flags value mismatch)
    /// ```
    fn matches_struct(
        ctx: &ParserCtx,
        syscall_struct: &crate::strace_parser::SyscallStruct,
        pattern_struct: &crate::scml_parser::PatternStruct,
    ) -> bool {
        // Without wildcard, field count must match exactly
        if !pattern_struct.wildcard()
            && syscall_struct.fields().len() != pattern_struct.fields().len()
        {
            return false;
        }

        // Every pattern field must be found and matched in the syscall struct
        for (pattern_field_name, pattern_field_value) in pattern_struct.fields() {
            match syscall_struct.get_field(pattern_field_name) {
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
