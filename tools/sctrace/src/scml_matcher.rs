// SPDX-License-Identifier: MPL-2.0

use crate::{
    scml_parser::{Pattern, Patterns},
    strace_parser::Syscall,
};

/// Matcher for syscalls against SCML patterns.
///
/// This struct provides functionality to match parsed syscalls from strace output
/// against pattern definitions loaded from SCML files.
pub struct Matcher {
    patterns: Patterns,
}

impl Matcher {
    /// Creates a new Matcher with the given patterns.
    ///
    /// # Arguments
    /// * `patterns` - Collection of SCML patterns organized by syscall name
    pub fn new(patterns: Patterns) -> Self {
        Self { patterns }
    }

    /// Attempts to match a syscall against available patterns.
    ///
    /// Returns the first pattern that matches the given syscall, or None if no match is found.
    /// Patterns are checked in the order they appear in the patterns collection.
    ///
    /// # Arguments
    /// * `syscall` - The parsed syscall to match against patterns
    ///
    /// # Returns
    /// * `Some(&Pattern)` - Reference to the first matching pattern
    /// * `None` - No pattern matched the syscall
    pub fn match_syscall(&self, syscall: &Syscall) -> Option<&Pattern> {
        let related_patterns = self.patterns.get(syscall.name());

        let related_patterns = match related_patterns {
            Some(p) => p,
            None => return None,
        };

        for pattern in related_patterns {
            if Self::matches(&syscall, &pattern) {
                return Some(&pattern);
            }
        }

        None
    }

    /// Matches a syscall against a specific pattern.
    ///
    /// # Matching Rules
    /// 1. Argument count must match exactly between syscall and pattern
    /// 2. Each argument is matched according to pattern type:
    ///    - `None` → Matches any syscall argument
    ///    - `Integer` → Matches syscall `Integer(_)` types
    ///    - `Path` → Not implemented (todo!)
    ///    - `Flag` → Should not appear (panics - flags should be wrapped in Flags)
    ///    - `Flags` → Matches syscall `Integer("0")` or `Flags`, where all syscall flags must be present in pattern flags
    ///    - `Array` → Matches syscall `Array`, recursively matching each element against pattern template
    ///    - `Struct` → Matches syscall `Struct` with field-by-field validation and optional wildcard support
    ///
    /// # Arguments
    /// * `syscall` - The syscall to match
    /// * `pattern` - The pattern to match against
    ///
    /// # Returns
    /// `true` if the syscall matches the pattern, `false` otherwise
    fn matches(syscall: &Syscall, pattern: &Pattern) -> bool {
        assert_eq!(syscall.name(), pattern.name());

        // 1. Argument count must match exactly if wildcard is not enabled
        if !pattern.wildcard() && syscall.args().len() != pattern.args().len() {
            return false;
        }

        // 2. Match each argument pair
        for (syscall_arg, pattern_arg) in syscall.args().iter().zip(pattern.args().iter()) {
            if !Self::matches_arg(syscall_arg, pattern_arg) {
                return false;
            }
        }

        true
    }

    /// Matches a single syscall argument against a pattern argument.
    ///
    /// # Pattern Matching Rules
    /// - `PatternArg::None` → Always matches
    /// - `PatternArg::Integer` → Matches only `SyscallArg::Integer(_)`
    /// - `PatternArg::Path` → Not implemented yet
    /// - `PatternArg::Flag(_)` → Invalid (should be wrapped in Flags)
    /// - `PatternArg::Flags` → Matches `Integer("0")` or validates flag subset
    /// - `PatternArg::Array` → Matches `Flag("NULL")` or recursively matches array elements
    /// - `PatternArg::Struct` → Matches `Flag("NULL")` or validates struct fields with optional wildcard
    ///
    /// # Arguments
    /// * `syscall_arg` - The syscall argument to match
    /// * `pattern_arg` - The pattern argument to match against
    ///
    /// # Returns
    /// `true` if the arguments match according to the pattern type
    fn matches_arg(
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

            // Single flags should not appear in patterns
            PatternArg::Flag(_) => {
                panic!(
                    "Single Flag should not appear in pattern matching - should be wrapped in Flags"
                )
            }

            // Flag set matching - supports both zero values and flag combinations
            PatternArg::Flags(pattern_flags) => match syscall_arg {
                // Special case: integer zero matches any flag pattern
                SyscallArg::Integer(value) if value == "0" => true,
                // Validate that all syscall flags are present in pattern
                SyscallArg::Flags(syscall_flags) => {
                    Self::matches_flags(syscall_flags, pattern_flags)
                }
                _ => false,
            },

            // Array matching - recursive element validation
            PatternArg::Array(pattern_array) => match syscall_arg {
                SyscallArg::Array(syscall_array) => {
                    Self::matches_array(syscall_array, pattern_array)
                }
                // Handle NULL array case - single Flag("NULL") matches any array pattern
                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    match flag_set.flags().get(0) {
                        Some(SyscallArg::Flag(flag_name)) if flag_name == "NULL" => true,
                        _ => false,
                    }
                }
                _ => false,
            },

            // Struct matching - field-by-field validation with wildcard support
            PatternArg::Struct(pattern_struct) => match syscall_arg {
                SyscallArg::Struct(syscall_struct) => {
                    Self::matches_struct(syscall_struct, pattern_struct)
                }
                // Handle NULL pointer case - single Flag("NULL") matches any struct pattern
                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    match flag_set.flags().get(0) {
                        Some(SyscallArg::Flag(flag_name)) if flag_name == "NULL" => true,
                        _ => false,
                    }
                }
                _ => false,
            },
        }
    }

    /// Matches syscall flags against pattern flags.
    ///
    /// Validates that every flag in the syscall flag set has a corresponding
    /// match in the pattern flag set. Supports both named flags and integer values.
    ///
    /// # Arguments
    /// * `syscall_flags` - The flag set from the syscall
    /// * `pattern_flags` - The flag set pattern to match against
    ///
    /// # Returns
    /// `true` if all syscall flags are matched by pattern flags
    fn matches_flags(
        syscall_flags: &crate::strace_parser::SyscallFlagSet,
        pattern_flags: &crate::scml_parser::PatternFlagSet,
    ) -> bool {
        use crate::{scml_parser::PatternArg, strace_parser::SyscallArg};

        // Every syscall flag must find a match in the pattern flags
        for syscall_flag in syscall_flags.flags() {
            let mut found_match = false;

            for pattern_flag in pattern_flags.flags() {
                match (syscall_flag, pattern_flag) {
                    // Named flag matching
                    (SyscallArg::Flag(syscall_name), PatternArg::Flag(pattern_name)) => {
                        if syscall_name == pattern_name {
                            found_match = true;
                            break;
                        }
                    }
                    // Integer flag matching (pattern accepts any integer)
                    (SyscallArg::Integer(_), PatternArg::Integer) => {
                        found_match = true;
                        break;
                    }
                    _ => {}
                }
            }

            if !found_match {
                return false;
            }
        }

        true
    }

    /// Matches syscall array against pattern array.
    ///
    /// Pattern arrays should contain exactly one element that serves as a template
    /// for matching all elements in the syscall array.
    ///
    /// # Arguments
    /// * `syscall_array` - The array from the syscall
    /// * `pattern_array` - The array pattern (should contain one template element)
    ///
    /// # Returns
    /// `true` if all syscall array elements match the pattern template
    fn matches_array(
        syscall_array: &crate::strace_parser::SyscallArray,
        pattern_array: &crate::scml_parser::PatternArray,
    ) -> bool {
        // Pattern array must have exactly one element as template
        if pattern_array.args().len() != 1 {
            return false;
        }

        let pattern_element = &pattern_array.args()[0];

        // Every syscall array element must match the pattern template
        for syscall_element in syscall_array.elements() {
            if !Self::matches_arg(syscall_element, pattern_element) {
                return false;
            }
        }

        true
    }

    /// Matches syscall struct against pattern struct.
    ///
    /// Validates struct field matching with support for wildcard fields.
    /// When wildcard is disabled, field counts must match exactly.
    /// When wildcard is enabled, syscall may contain additional fields.
    ///
    /// # Arguments
    /// * `syscall_struct` - The struct from the syscall
    /// * `pattern_struct` - The struct pattern with field constraints
    ///
    /// # Returns
    /// `true` if all pattern fields are found and matched in the syscall struct
    fn matches_struct(
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
                    if !Self::matches_arg(syscall_field_value, pattern_field_value) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{scml_parser::*, strace_parser::*};

    /// Helper function to create a basic matcher with predefined patterns
    fn create_test_matcher() -> Matcher {
        let mut patterns = HashMap::new();

        // Basic patterns for testing
        patterns.insert(
            "test_syscall".to_string(),
            vec![Pattern::new(
                "test_syscall".to_string(),
                vec![
                    PatternArg::None,
                    PatternArg::Integer,
                    PatternArg::Flags(PatternFlagSet::new(vec![
                        PatternArg::Flag("FLAG1".to_string()),
                        PatternArg::Flag("FLAG2".to_string()),
                    ])),
                ],
                false,
            )],
        );

        patterns.insert(
            "open".to_string(),
            vec![Pattern::new(
                "open".to_string(),
                vec![
                    PatternArg::None, // path
                    PatternArg::Flags(PatternFlagSet::new(vec![
                        PatternArg::Flag("O_RDONLY".to_string()),
                        PatternArg::Flag("O_WRONLY".to_string()),
                        PatternArg::Flag("O_RDWR".to_string()),
                    ])),
                    PatternArg::Integer, // mode
                ],
                false,
            )],
        );

        Matcher::new(Patterns::new(patterns))
    }

    #[test]
    fn test_match_syscall_no_patterns_for_syscall() {
        let matcher = create_test_matcher();

        let syscall = Syscall::new(
            1234,
            "nonexistent_syscall".to_string(),
            vec![SyscallArg::Integer("42".to_string())],
            "0".to_string(),
            "nonexistent_syscall(42) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_none());
    }

    #[test]
    fn test_match_syscall_successful_match() {
        let matcher = create_test_matcher();

        let syscall = Syscall::new(
            1234,
            "test_syscall".to_string(),
            vec![
                SyscallArg::String("any_string".to_string()),
                SyscallArg::Integer("42".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("FLAG1".to_string())]).unwrap(),
                ),
            ],
            "0".to_string(),
            "test_syscall(\"any_string\", 42, FLAG1) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "test_syscall");
    }

    #[test]
    fn test_match_syscall_argument_count_mismatch() {
        let matcher = create_test_matcher();

        // Syscall with wrong number of arguments
        let syscall = Syscall::new(
            1234,
            "test_syscall".to_string(),
            vec![
                SyscallArg::String("any_string".to_string()),
                SyscallArg::Integer("42".to_string()),
                // Missing third argument
            ],
            "0".to_string(),
            "test_syscall(\"any_string\", 42) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_none());
    }

    #[test]
    fn test_matches_arg_none_pattern() {
        // PatternArg::None should match any syscall argument
        assert!(Matcher::matches_arg(
            &SyscallArg::Integer("123".to_string()),
            &PatternArg::None,
        ));

        assert!(Matcher::matches_arg(
            &SyscallArg::String("test".to_string()),
            &PatternArg::None,
        ));

        assert!(Matcher::matches_arg(
            &SyscallArg::Flag("FLAG".to_string()),
            &PatternArg::None,
        ));
    }

    #[test]
    fn test_matches_arg_integer_pattern() {
        // PatternArg::Integer should only match SyscallArg::Integer
        assert!(Matcher::matches_arg(
            &SyscallArg::Integer("123".to_string()),
            &PatternArg::Integer,
        ));

        assert!(!Matcher::matches_arg(
            &SyscallArg::String("123".to_string()),
            &PatternArg::Integer,
        ));

        assert!(!Matcher::matches_arg(
            &SyscallArg::Flag("FLAG".to_string()),
            &PatternArg::Integer,
        ));
    }

    #[test]
    #[should_panic(expected = "Single Flag should not appear in pattern matching")]
    fn test_matches_arg_single_flag_panics() {
        Matcher::matches_arg(
            &SyscallArg::Flag("TEST".to_string()),
            &PatternArg::Flag("TEST".to_string()),
        );
    }

    #[test]
    fn test_matches_arg_flags_pattern_with_zero() {
        // Integer "0" should match any flag pattern
        let pattern_flags = PatternFlagSet::new(vec![
            PatternArg::Flag("FLAG1".to_string()),
            PatternArg::Flag("FLAG2".to_string()),
        ]);

        assert!(Matcher::matches_arg(
            &SyscallArg::Integer("0".to_string()),
            &PatternArg::Flags(pattern_flags),
        ));
    }

    #[test]
    fn test_matches_arg_flags_pattern_with_syscall_flags() {
        let pattern_flags = PatternFlagSet::new(vec![
            PatternArg::Flag("FLAG1".to_string()),
            PatternArg::Flag("FLAG2".to_string()),
            PatternArg::Integer,
        ]);

        let syscall_flags = SyscallFlagSet::new(vec![
            SyscallArg::Flag("FLAG1".to_string()),
            SyscallArg::Integer("123".to_string()),
        ])
        .unwrap();

        assert!(Matcher::matches_arg(
            &SyscallArg::Flags(syscall_flags),
            &PatternArg::Flags(pattern_flags),
        ));
    }

    #[test]
    fn test_matches_arg_flags_pattern_mismatch() {
        let pattern_flags = PatternFlagSet::new(vec![PatternArg::Flag("FLAG1".to_string())]);

        let syscall_flags = SyscallFlagSet::new(vec![
            SyscallArg::Flag("FLAG2".to_string()), // Different flag
        ])
        .unwrap();

        assert!(!Matcher::matches_arg(
            &SyscallArg::Flags(syscall_flags),
            &PatternArg::Flags(pattern_flags),
        ));
    }

    #[test]
    fn test_matches_arg_array_pattern() {
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);

        let syscall_array = SyscallArray::new(vec![
            SyscallArg::Integer("1".to_string()),
            SyscallArg::Integer("2".to_string()),
            SyscallArg::Integer("3".to_string()),
        ])
        .unwrap();

        assert!(Matcher::matches_arg(
            &SyscallArg::Array(syscall_array),
            &PatternArg::Array(pattern_array),
        ));
    }

    #[test]
    fn test_matches_arg_array_pattern_wrong_element_type() {
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);

        let syscall_array =
            SyscallArray::new(vec![SyscallArg::String("not_integer".to_string())]).unwrap();

        assert!(!Matcher::matches_arg(
            &SyscallArg::Array(syscall_array),
            &PatternArg::Array(pattern_array),
        ));
    }

    #[test]
    fn test_matches_arg_struct_pattern() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("field1".to_string(), PatternArg::Integer);
        pattern_fields.insert("field2".to_string(), PatternArg::None);
        let pattern_struct = PatternStruct::new(pattern_fields, false);

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert("field1".to_string(), SyscallArg::Integer("123".to_string()));
        syscall_fields.insert("field2".to_string(), SyscallArg::String("test".to_string()));
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(Matcher::matches_arg(
            &SyscallArg::Struct(syscall_struct),
            &PatternArg::Struct(pattern_struct),
        ));
    }

    #[test]
    fn test_matches_arg_struct_pattern_with_wildcard() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("required_field".to_string(), PatternArg::Integer);
        let pattern_struct = PatternStruct::new(pattern_fields, true); // wildcard enabled

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert(
            "required_field".to_string(),
            SyscallArg::Integer("123".to_string()),
        );
        syscall_fields.insert(
            "extra_field".to_string(),
            SyscallArg::String("extra".to_string()),
        );
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(Matcher::matches_arg(
            &SyscallArg::Struct(syscall_struct),
            &PatternArg::Struct(pattern_struct),
        ));
    }

    #[test]
    fn test_matches_arg_struct_pattern_without_wildcard_extra_fields() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("field1".to_string(), PatternArg::Integer);
        let pattern_struct = PatternStruct::new(pattern_fields, false); // wildcard disabled

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert("field1".to_string(), SyscallArg::Integer("123".to_string()));
        syscall_fields.insert(
            "extra_field".to_string(),
            SyscallArg::String("extra".to_string()),
        );
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(!Matcher::matches_arg(
            &SyscallArg::Struct(syscall_struct),
            &PatternArg::Struct(pattern_struct),
        ));
    }

    #[test]
    fn test_matches_arg_struct_pattern_missing_required_field() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("required_field".to_string(), PatternArg::Integer);
        let pattern_struct = PatternStruct::new(pattern_fields, true);

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert(
            "other_field".to_string(),
            SyscallArg::String("test".to_string()),
        );
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(!Matcher::matches_arg(
            &SyscallArg::Struct(syscall_struct),
            &PatternArg::Struct(pattern_struct),
        ));
    }

    #[test]
    fn test_matches_flags_all_syscall_flags_match() {
        let pattern_flags = PatternFlagSet::new(vec![
            PatternArg::Flag("FLAG1".to_string()),
            PatternArg::Flag("FLAG2".to_string()),
            PatternArg::Integer,
        ]);

        let syscall_flags = SyscallFlagSet::new(vec![
            SyscallArg::Flag("FLAG1".to_string()),
            SyscallArg::Integer("123".to_string()),
        ])
        .unwrap();

        assert!(Matcher::matches_flags(&syscall_flags, &pattern_flags));
    }

    #[test]
    fn test_matches_flags_syscall_flag_not_in_pattern() {
        let pattern_flags = PatternFlagSet::new(vec![PatternArg::Flag("FLAG1".to_string())]);

        let syscall_flags = SyscallFlagSet::new(vec![
            SyscallArg::Flag("FLAG1".to_string()),
            SyscallArg::Flag("FLAG_NOT_IN_PATTERN".to_string()),
        ])
        .unwrap();

        assert!(!Matcher::matches_flags(&syscall_flags, &pattern_flags));
    }

    #[test]
    fn test_matches_array_single_template_element() {
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);

        let syscall_array = SyscallArray::new(vec![
            SyscallArg::Integer("1".to_string()),
            SyscallArg::Integer("2".to_string()),
            SyscallArg::Integer("3".to_string()),
        ])
        .unwrap();

        assert!(Matcher::matches_array(&syscall_array, &pattern_array));
    }

    #[test]
    fn test_matches_array_multiple_template_elements_fails() {
        let pattern_array = PatternArray::new(vec![
            PatternArg::Integer,
            PatternArg::Integer, // Pattern should have only one template element
        ]);

        let syscall_array = SyscallArray::new(vec![SyscallArg::Integer("1".to_string())]).unwrap();

        assert!(!Matcher::matches_array(&syscall_array, &pattern_array));
    }

    #[test]
    fn test_matches_array_element_mismatch() {
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);

        let syscall_array =
            SyscallArray::new(vec![SyscallArg::String("not_integer".to_string())]).unwrap();

        assert!(!Matcher::matches_array(&syscall_array, &pattern_array));
    }

    #[test]
    fn test_matches_struct_exact_field_count_without_wildcard() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("field1".to_string(), PatternArg::Integer);
        pattern_fields.insert("field2".to_string(), PatternArg::None);
        let pattern_struct = PatternStruct::new(pattern_fields, false);

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert("field1".to_string(), SyscallArg::Integer("123".to_string()));
        syscall_fields.insert("field2".to_string(), SyscallArg::String("test".to_string()));
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(Matcher::matches_struct(&syscall_struct, &pattern_struct));
    }

    #[test]
    fn test_matches_struct_field_value_mismatch() {
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("field1".to_string(), PatternArg::Integer);
        let pattern_struct = PatternStruct::new(pattern_fields, false);

        let mut syscall_fields = HashMap::new();
        syscall_fields.insert(
            "field1".to_string(),
            SyscallArg::String("not_integer".to_string()),
        );
        let syscall_struct = SyscallStruct::new(syscall_fields);

        assert!(!Matcher::matches_struct(&syscall_struct, &pattern_struct));
    }

    #[test]
    fn test_integration_complex_syscall_matching() {
        // Create a complex pattern for testing integration
        let mut patterns = HashMap::new();

        // Create a pattern for openat syscall
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "st_mode".to_string(),
            PatternArg::Flags(PatternFlagSet::new(vec![
                PatternArg::Flag("S_IFDIR".to_string()),
                PatternArg::Integer,
            ])),
        );
        let stat_pattern = PatternStruct::new(struct_fields, true);

        patterns.insert(
            "stat".to_string(),
            vec![Pattern::new(
                "stat".to_string(),
                vec![
                    PatternArg::None, // path
                    PatternArg::Struct(stat_pattern),
                ],
                false,
            )],
        );

        let patterns = Patterns::new(patterns);

        let matcher = Matcher::new(patterns);

        // Create a matching syscall
        let mut syscall_fields = HashMap::new();
        syscall_fields.insert(
            "st_mode".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("S_IFDIR".to_string()),
                    SyscallArg::Integer("0755".to_string()),
                ])
                .unwrap(),
            ),
        );
        syscall_fields.insert(
            "st_size".to_string(),
            SyscallArg::Integer("4096".to_string()),
        );

        let syscall = Syscall::new(
            1234,
            "stat".to_string(),
            vec![
                SyscallArg::String("/path/to/file".to_string()),
                SyscallArg::Struct(SyscallStruct::new(syscall_fields)),
            ],
            "0".to_string(),
            "stat(\"/path/to/file\", {st_mode=S_IFDIR|0755, st_size=4096}) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "stat");
    }

    #[test]
    fn test_integration_multiple_patterns_first_match() {
        // Test that the first matching pattern is returned
        let mut patterns = HashMap::new();

        patterns.insert(
            "test".to_string(),
            vec![
                Pattern::new("test".to_string(), vec![PatternArg::Integer], false),
                Pattern::new("test".to_string(), vec![PatternArg::None], false),
            ],
        );

        let patterns = Patterns::new(patterns);

        let matcher = Matcher::new(patterns);

        let syscall = Syscall::new(
            1234,
            "test".to_string(),
            vec![SyscallArg::Integer("42".to_string())],
            "0".to_string(),
            "test(42) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_some());

        // Should match the first pattern (Integer constraint)
        assert_eq!(result.unwrap().args().len(), 1);
        assert!(matches!(result.unwrap().args()[0], PatternArg::Integer));
    }

    #[test]
    fn test_real_world_openat_example() {
        // Test with a real-world openat syscall example
        let mut patterns = HashMap::new();

        patterns.insert(
            "openat".to_string(),
            vec![Pattern::new(
                "openat".to_string(),
                vec![
                    PatternArg::None, // dirfd
                    PatternArg::None, // TODO: replace with Path limitation
                    PatternArg::Flags(PatternFlagSet::new(vec![
                        PatternArg::Flag("O_RDONLY".to_string()),
                        PatternArg::Flag("O_CLOEXEC".to_string()),
                    ])),
                    PatternArg::Integer, // mode
                ],
                false,
            )],
        );

        let patterns = Patterns::new(patterns);

        let matcher = Matcher::new(patterns);

        let syscall = Syscall::new(
            1234,
            "openat".to_string(),
            vec![
                SyscallArg::FdPath("/home/user".to_string()),
                SyscallArg::String("file.txt".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("O_RDONLY".to_string())]).unwrap(),
                ),
                SyscallArg::Integer("0644".to_string()),
            ],
            "3".to_string(),
            "openat(AT_FDCWD</home/user>, \"file.txt\", O_RDONLY, 0644) = 3".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        assert!(result.is_some());
        //assert_eq!(result.unwrap().name(), "openat");
    }

    #[test]
    fn test_path_pattern_todo() {
        // Test that Path pattern matching is not yet implemented
        let result = std::panic::catch_unwind(|| {
            Matcher::matches_arg(
                &SyscallArg::String("/path/to/file".to_string()),
                &PatternArg::Path,
            )
        });

        assert!(result.is_err());
    }

    #[test]
    fn test_empty_arrays_match() {
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);
        let syscall_array = SyscallArray::new(vec![]).unwrap();

        // Empty array should match (no elements to validate against template)
        assert!(Matcher::matches_array(&syscall_array, &pattern_array));
    }

    #[test]
    fn test_empty_structs_match() {
        let pattern_struct = PatternStruct::new(HashMap::new(), false);
        let syscall_struct = SyscallStruct::new(HashMap::new());

        // Empty structs should match
        assert!(Matcher::matches_struct(&syscall_struct, &pattern_struct));
    }

    #[test]
    fn test_matches_arg_struct_pattern_null_flag() {
        // Test that single Flag("NULL") matches any struct pattern
        let mut pattern_fields = HashMap::new();
        pattern_fields.insert("field1".to_string(), PatternArg::Integer);
        let pattern_struct = PatternStruct::new(pattern_fields, false);

        let null_flag_set =
            SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap();

        assert!(Matcher::matches_arg(
            &SyscallArg::Flags(null_flag_set),
            &PatternArg::Struct(pattern_struct),
        ));
    }

    #[test]
    fn test_matches_arg_array_pattern_null_flag() {
        // Test that single Flag("NULL") matches any array pattern
        let pattern_array = PatternArray::new(vec![PatternArg::Integer]);

        let null_flag_set =
            SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap();

        assert!(Matcher::matches_arg(
            &SyscallArg::Flags(null_flag_set),
            &PatternArg::Array(pattern_array),
        ));
    }

    #[test]
    fn test_pattern_wildcard_enabled_extra_syscall_args() {
        // Pattern with wildcard enabled: should match syscalls with extra arguments
        let mut patterns = HashMap::new();
        patterns.insert(
            "wild_test".to_string(),
            vec![Pattern::new(
                "wild_test".to_string(),
                vec![PatternArg::Integer, PatternArg::None],
                true, // wildcard enabled, allows extra syscall arguments
            )],
        );
        let patterns = Patterns::new(patterns);
        let matcher = Matcher::new(patterns);

        // Syscall has more arguments than pattern, but wildcard allows it
        let syscall = Syscall::new(
            1,
            "wild_test".to_string(),
            vec![
                SyscallArg::Integer("1".to_string()),
                SyscallArg::String("any".to_string()),
                SyscallArg::Flag("EXTRA".to_string()),
            ],
            "0".to_string(),
            "wild_test(1, \"any\", EXTRA) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        // Should match because wildcard is enabled
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "wild_test");
    }

    #[test]
    fn test_pattern_wildcard_disabled_extra_syscall_args() {
        // Pattern with wildcard disabled: should NOT match syscalls with extra arguments
        let mut patterns = HashMap::new();
        patterns.insert(
            "wild_test".to_string(),
            vec![Pattern::new(
                "wild_test".to_string(),
                vec![PatternArg::Integer, PatternArg::None],
                false, // wildcard disabled, argument count must match exactly
            )],
        );
        let patterns = Patterns::new(patterns);
        let matcher = Matcher::new(patterns);

        // Syscall has more arguments than pattern, should not match
        let syscall = Syscall::new(
            1,
            "wild_test".to_string(),
            vec![
                SyscallArg::Integer("1".to_string()),
                SyscallArg::String("any".to_string()),
                SyscallArg::Flag("EXTRA".to_string()),
            ],
            "0".to_string(),
            "wild_test(1, \"any\", EXTRA) = 0".to_string(),
        );

        let result = matcher.match_syscall(&syscall);
        // Should not match because wildcard is disabled and argument count mismatches
        assert!(result.is_none());
    }
}
