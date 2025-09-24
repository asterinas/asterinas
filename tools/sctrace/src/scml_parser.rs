// SPDX-License-Identifier: MPL-2.0

//! SCML (System Call Matching Language) parser module.
//!
//! This module provides functionality to parse SCML files, which define patterns
//! for matching system call invocations. SCML is a domain-specific language that
//! allows specifying constraints on syscall arguments, including support for:
//!
//! - Primitive types (integers, paths)
//! - Flags and flag combinations
//! - Structured data with named fields
//! - Arrays with type consistency
//! - Variable definitions for reusable patterns
//! - Wildcard matching for flexible constraints
//!
//! # SCML Syntax
//!
//! ## Pattern Definitions
//!
//! ```text
//! syscall_name(arg1 = constraint1, arg2 = constraint2, ..);
//! ```
//!
//! ## Type Constraints
//!
//! - `<INTEGER>` - Matches any integer value
//! - `<PATH>` - Matches file path strings
//! - `FLAG_NAME` - Matches a specific flag
//! - `FLAG1 | FLAG2` - Matches flag combinations
//! - `{field1 = value1, field2 = value2}` - Matches struct fields
//! - `[element1, element2]` - Matches array elements
//!
//! ## Variable Definitions
//!
//! ```text
//! access_mode = O_RDONLY | O_WRONLY | O_RDWR;
//! struct sigaction = { sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT, .. };
//! ```
//!
//! ## Wildcard Support
//!
//! - `..` in argument list - Accepts additional unspecified arguments
//! - `..` in struct - Accepts additional unspecified fields
//!
//! # Example
//!
//! ```text
//! // Define reusable flag sets
//! access_mode = O_RDONLY | O_WRONLY | O_RDWR;
//!
//! // Pattern with constraints
//! openat(fd = AT_FDCWD, filename = <PATH>, flags = <access_mode>, ..);
//!
//! // Pattern with struct constraint
//! rt_sigaction(sig = <INTEGER>, act = {sa_flags = SA_NOCLDSTOP, ..});
//! ```
//!
//! # Usage Example
//!
//! ```text
//! use scml_parser::Patterns;
//!
//! // Parse SCML file
//! let patterns = Patterns::from_scml_file("syscalls.scml")
//!     .expect("Failed to parse SCML file");
//!
//! // Look up patterns for a specific syscall
//! if let Some(openat_patterns) = patterns.get("openat") {
//!     println!("Found {} patterns for openat", openat_patterns.len());
//! }
//! ```

use std::{collections::HashMap, error::Error, fmt, fs};

use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, take_while},
    character::complete::{char, multispace0},
    combinator::{opt, recognize},
    multi::{separated_list0, separated_list1},
    sequence::{delimited, pair, preceded},
};

/// Custom error type for SCML parsing operations.
///
/// This enum represents various error conditions that can occur during
/// SCML file reading and parsing.
#[derive(Debug, Clone)]
pub enum ScmlParseError {
    /// File I/O error when reading SCML files.
    ///
    /// Contains a descriptive message including the file path and error details.
    IoError(String),

    /// Parsing error with details.
    ///
    /// Contains information about what failed to parse and where.
    ParseError(String),

    /// Incomplete statement.
    ///
    /// Occurs when a statement doesn't end with a semicolon before EOF.
    IncompleteStatement(String),
}

impl fmt::Display for ScmlParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScmlParseError::IoError(msg) => write!(f, "I/O error: {}", msg),
            ScmlParseError::ParseError(msg) => write!(f, "Parsing error: {}", msg),
            ScmlParseError::IncompleteStatement(stmt) => {
                write!(f, "Incomplete statement: {}", stmt)
            }
        }
    }
}

impl Error for ScmlParseError {}

/// Represents different types of pattern arguments with their constraints.
///
/// Pattern arguments define matching rules for syscall arguments. They can
/// represent simple types, complex structures, or references to variables
/// defined earlier in the SCML file.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternArg<'a> {
    /// No constraints - accepts any value.
    ///
    /// Used when an argument should match any value without restriction.
    None,

    /// Must be an integer type.
    ///
    /// Matches numeric arguments including decimal and hexadecimal values.
    Integer,

    /// Must match a file path pattern.
    ///
    /// Matches string arguments representing file paths.
    Path,

    /// Must match a single flag value.
    ///
    /// Matches a specific named constant or flag.
    Flag(&'a str),

    /// Array of pattern arguments.
    ///
    /// Matches array arguments where each element matches the corresponding
    /// pattern in the array.
    Array(PatternArray<'a>),

    /// Structured type with named fields and optional wildcard matching.
    ///
    /// Matches struct arguments with specified field constraints.
    Struct(PatternStruct<'a>),

    /// Set of flags combinable with bitwise OR operations.
    ///
    /// Matches flag combinations like `O_RDWR | O_CREAT`.
    Flags(PatternFlagSet<'a>),

    /// Reference to a flags variable defined earlier.
    ///
    /// Contains the variable ID that resolves to a `PatternFlagSet`.
    FlagsVariable(&'a str),

    /// Reference to a struct variable defined earlier.
    ///
    /// Contains the variable ID that resolves to a `PatternStruct`
    /// or `PatternMultipleStruct`.
    StructVariable(&'a str),

    /// Multiple struct alternatives.
    ///
    /// Matches if any of the struct patterns match. Used when a struct
    /// variable is defined multiple times with different field combinations.
    MultipleStruct(PatternMultipleStruct<'a>),
}

impl<'a> PatternArg<'a> {
    /// Resolves a variable reference to its actual pattern definition.
    ///
    /// This method dereferences `FlagsVariable` and `StructVariable` types
    /// to retrieve their actual pattern definitions from the parser context.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context containing variable definitions
    ///
    /// # Returns
    ///
    /// Reference to the resolved pattern argument
    ///
    /// # Panics
    ///
    /// Panics if called on a non-variable type or if the variable is not found
    /// in the context.
    ///
    /// # Examples
    ///
    /// ```text
    /// # use scml_parser::{PatternArg, ParserCtx};
    /// # let ctx = ParserCtx::new();
    /// # let arg = PatternArg::None;
    /// match arg {
    ///     PatternArg::FlagsVariable(_) | PatternArg::StructVariable(_) => {
    ///         let resolved = arg.get(&ctx);
    ///         // Use resolved pattern
    ///     }
    ///     _ => {
    ///         // Direct pattern, no resolution needed
    ///     }
    /// }
    /// ```
    pub fn get<'b>(&self, ctx: &'b ParserCtx) -> &'b PatternArg<'b> {
        match self {
            PatternArg::FlagsVariable(id) => ctx.flags_lookup(id).unwrap(),

            PatternArg::StructVariable(id) => ctx.struct_lookup(id).unwrap(),

            _ => {
                panic!("get() can only be called on variable reference types");
            }
        }
    }
}

/// Represents an array pattern constraint.
///
/// An array pattern matches array arguments where each element must match
/// the corresponding pattern in the sequence.
///
/// # Examples
///
/// In SCML:
/// ```text
/// // Array with integer elements
/// poll(fds = [ <INTEGER> ], nfds, timeout);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PatternArray<'a>(Vec<PatternArg<'a>>);

/// Represents a flag set pattern constraint.
///
/// A flag set matches bitwise OR combinations of flags. All elements must be
/// either flag identifiers or integer types that can be combined with `|`.
///
/// # Examples
///
/// In SCML:
/// ```text
/// // Flag combination
/// open(path, flags = O_RDWR | O_CREAT | O_EXCL);
///
/// // Mixed flags and integers
/// timer_create(clockid = CLOCK_PROCESS_CPUTIME_ID | <INTEGER>, sevp, timerid);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PatternFlagSet<'a>(Vec<PatternArg<'a>>);

/// Represents a struct pattern constraint.
///
/// A struct pattern matches structured data with named fields. It can specify
/// exact field matching or allow additional fields via wildcard.
///
/// # Fields
///
/// * `0` - HashMap of field name to pattern constraint
/// * `1` - Wildcard flag (if `true`, allows additional fields)
///
/// # Examples
///
/// In SCML:
/// ```text
/// // Exact struct match
/// stat(statbuf = {st_mode = S_IFREG, st_size = <INTEGER>});
///
/// // Struct with wildcard (allows extra fields)
/// sigaction(act = {sa_flags = SA_RESTART, ..});
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PatternStruct<'a>(HashMap<&'a str, PatternArg<'a>>, bool);

/// Represents multiple alternative struct patterns.
///
/// This is used when a struct variable is defined multiple times with different
/// field combinations. The pattern matches if any of the alternatives match.
///
/// # Examples
///
/// In SCML:
/// ```text
/// // First definition
/// struct sigaction = {sa_flags = SA_RESTART, ..};
/// // Second definition (same name) - creates alternatives
/// struct sigaction = {sa_flags = SA_NOCLDSTOP, ..};
///
/// // Usage matches either alternative
/// rt_sigaction(signum, act = <sigaction>, oldact = <sigaction>);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct PatternMultipleStruct<'a>(Vec<PatternArg<'a>>);

impl<'a> PatternMultipleStruct<'a> {
    /// Creates a new PatternMultipleStruct.
    ///
    /// # Arguments
    ///
    /// * `structs` - Vector of struct patterns (alternatives)
    ///
    /// # Panics
    ///
    /// Panics if any element is not a `PatternArg::Struct` variant.
    pub fn new(structs: Vec<PatternArg<'a>>) -> Self {
        for struct_arg in &structs {
            match struct_arg {
                PatternArg::Struct(_) => {
                    // Valid struct type
                }
                _ => {
                    panic!("PatternMultipleStruct can only contain Struct types");
                }
            }
        }
        Self(structs)
    }

    /// Returns a reference to the struct alternatives.
    pub fn structs(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }
}

impl<'a> PatternArray<'a> {
    /// Creates a new array pattern.
    ///
    /// # Arguments
    ///
    /// * `args` - Vector of pattern arguments representing array elements
    ///
    /// # Examples
    ///
    /// ```text
    /// # use scml_parser::{PatternArray, PatternArg};
    /// let array = PatternArray::new(vec![
    ///     PatternArg::Integer,
    ///     PatternArg::Path,
    /// ]);
    /// ```
    pub fn new(args: Vec<PatternArg<'a>>) -> Self {
        Self(args)
    }

    /// Returns a reference to the array element patterns.
    pub fn args(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }
}

impl<'a> PatternFlagSet<'a> {
    /// Creates a new flag set pattern.
    ///
    /// # Arguments
    ///
    /// * `flags` - Vector of flag patterns (must be Flag, Integer, or FlagsVariable)
    ///
    /// # Panics
    ///
    /// Panics if any element is not a valid flag type (Flag, Integer, or FlagsVariable).
    ///
    /// # Examples
    ///
    /// ```text
    /// # use scml_parser::{PatternFlagSet, PatternArg};
    /// let flags = PatternFlagSet::new(vec![
    ///     PatternArg::Flag("O_RDWR"),
    ///     PatternArg::Flag("O_CREAT"),
    /// ]);
    /// ```
    pub fn new(flags: Vec<PatternArg<'a>>) -> Self {
        // Validate that all elements are either Flag or Integer types
        for flag in &flags {
            match flag {
                PatternArg::Flag(_) | PatternArg::Integer | PatternArg::FlagsVariable(_) => {
                    // Valid flag type
                }
                _ => {
                    panic!("PatternFlagSet can only contain Flag or Integer types");
                }
            }
        }

        Self(flags)
    }

    /// Returns a reference to the flag elements.
    pub fn flags(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }
}

impl<'a> PatternStruct<'a> {
    /// Creates a new struct pattern.
    ///
    /// # Arguments
    ///
    /// * `fields` - HashMap mapping field names to their pattern constraints
    /// * `wildcard` - Whether to accept additional unspecified fields
    ///
    /// # Examples
    ///
    /// ```text
    /// # use scml_parser::{PatternStruct, PatternArg};
    /// # use std::collections::HashMap;
    /// let mut fields = HashMap::new();
    /// fields.insert("sa_flags", PatternArg::Flag("SA_RESTART"));
    /// let pattern = PatternStruct::new(fields, true);
    /// ```
    pub fn new(fields: HashMap<&'a str, PatternArg<'a>>, wildcard: bool) -> Self {
        Self(fields, wildcard)
    }

    /// Returns a reference to the struct field patterns.
    pub fn fields(&self) -> &HashMap<&'a str, PatternArg<'a>> {
        &self.0
    }

    /// Returns whether this struct pattern accepts wildcard fields.
    ///
    /// If `true`, the struct can have additional fields beyond those specified.
    /// If `false`, only the explicitly mentioned fields are allowed.
    pub fn wildcard(&self) -> bool {
        self.1
    }
}

/// Represents a complete syscall pattern with constraints.
///
/// A pattern defines matching rules for a specific syscall, including
/// constraints on its arguments and whether it accepts additional parameters.
///
/// # Examples
///
/// In SCML:
/// ```text
/// // Exact argument match
/// write(fd = <INTEGER>, buf = <PATH>, count = <INTEGER>);
///
/// // With wildcard (accepts extra arguments)
/// open(filename = <PATH>, flags = O_RDONLY, ..);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern<'a> {
    /// Name of the pattern (corresponds to syscall name).
    name: &'a str,

    /// Ordered list of argument patterns for this syscall.
    args: Vec<PatternArg<'a>>,

    /// Whether this pattern accepts additional unspecified arguments.
    wildcard: bool,
}

impl<'a> Pattern<'a> {
    /// Parses a complete pattern definition from SCML source.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context with variable definitions
    /// * `input` - SCML source string to parse
    ///
    /// # Returns
    ///
    /// `IResult` containing remaining input and parsed `Pattern` on success.
    ///
    /// # Examples
    ///
    /// ```text
    /// # use scml_parser::{Pattern, ParserCtx};
    /// # let ctx = ParserCtx::new();
    /// let input = "open(filename = <PATH>, flags = O_RDONLY);";
    /// let result = Pattern::parse(&ctx, input);
    /// ```
    pub fn parse(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, Pattern<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('(')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, args) = opt(|i| Self::parse_param_list(ctx, i))(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = opt(char(','))(input)?; // Allow trailing comma
        let (input, has_wildcard) = opt(preceded(
            multispace0,
            delimited(multispace0, tag(".."), multispace0),
        ))(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(')')(input)?;
        let (input, _) = char(';')(input)?;

        let args = args.unwrap_or_default();

        Ok((input, Pattern::new(name, args, has_wildcard.is_some())))
    }

    pub fn new(name: &'a str, args: Vec<PatternArg<'a>>, wildcard: bool) -> Self {
        Self {
            name,
            args,
            wildcard,
        }
    }

    /// Returns the pattern name (syscall name).
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns a reference to the pattern arguments.
    pub fn args(&self) -> &Vec<PatternArg<'a>> {
        &self.args
    }

    /// Parses a comma-separated list of parameters.
    ///
    /// # Format
    ///
    /// ```text
    /// param1 = constraint1, param2 = constraint2, param3
    /// ```
    fn parse_param_list(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, Vec<PatternArg<'a>>> {
        separated_list0(delimited(multispace0, char(','), multispace0), |i| {
            Self::parse_param(ctx, i)
        })(input)
    }

    /// Parses a single parameter with optional constraint.
    ///
    /// # Format
    ///
    /// ```text
    /// param_name = constraint
    /// param_name              // Unconstrained (matches any value)
    /// ```
    fn parse_param(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = identifier(input)?;
        let (input, _) = multispace0(input)?;

        // Check if parameter has constraint (= value)
        if let Ok((input, _)) = char::<&str, nom::error::Error<&str>>('=')(input) {
            let (input, _) = multispace0(input)?;
            Self::parse_expr(ctx, input)
        } else {
            // Unconstrained parameter
            Ok((input, PatternArg::None))
        }
    }

    /// Parses an expression (constraint) for a parameter.
    ///
    /// Expressions can be structs, arrays, built-in types, flags, or variables.
    fn parse_expr(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        alt((
            |i| Self::parse_struct(ctx, i),
            |i| Self::parse_array(ctx, i),
            Self::parse_builtin_type,
            |i| Self::parse_flags(ctx, i),
            |i| Self::parse_struct_variable(ctx, i),
            |i| Self::parse_flags_variable(ctx, i),
        ))(input)
    }

    /// Parses a struct pattern.
    ///
    /// # Format
    ///
    /// ```text
    /// {field1 = value1, field2 = value2}       // Exact match
    /// {field1 = value1, ..}                    // With wildcard
    /// ```
    fn parse_struct(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('{')(input)?;
        let (input, _) = multispace0(input)?;

        let (input, fields) =
            separated_list0(delimited(multispace0, char(','), multispace0), |i| {
                Self::parse_struct_field(ctx, i)
            })(input)?;

        let (input, _) = multispace0(input)?;

        // Check for wildcard (..)
        let (input, wildcard) = opt(preceded(
            opt(char(',')),
            delimited(multispace0, tag(".."), multispace0),
        ))(input)?;

        let (input, _) = multispace0(input)?;
        let (input, _) = char('}')(input)?;

        let mut field_map = HashMap::new();
        for (name, arg) in fields {
            field_map.insert(name, arg);
        }

        let has_wildcard = wildcard.is_some();
        Ok((
            input,
            PatternArg::Struct(PatternStruct::new(field_map, has_wildcard)),
        ))
    }

    /// Parses a struct field definition.
    ///
    /// # Format
    ///
    /// ```text
    /// field_name = expression
    /// field_name              // Unconstrained field
    /// ```
    fn parse_struct_field(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, (&'a str, PatternArg<'a>)> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;

        if let Ok((input, _)) = char::<&str, nom::error::Error<&str>>('=')(input) {
            let (input, _) = multispace0(input)?;
            let (input, expr) = Self::parse_expr(ctx, input)?;
            Ok((input, (name, expr)))
        } else {
            Ok((input, (name, PatternArg::None)))
        }
    }

    /// Parses an array pattern.
    ///
    /// # Format
    ///
    /// ```text
    /// [element1, element2, element3]
    /// ```
    fn parse_array(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('[')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, elements) =
            separated_list0(delimited(multispace0, char(','), multispace0), |i| {
                Self::parse_expr(ctx, i)
            })(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(']')(input)?;

        Ok((input, PatternArg::Array(PatternArray::new(elements))))
    }

    /// Parses a flags pattern (bitwise OR combination).
    ///
    /// # Format
    ///
    /// ```text
    /// FLAG1 | FLAG2 | FLAG3
    /// O_RDONLY | O_WRONLY
    /// 0x1 | 0x2 | FLAG_NAME
    /// ```
    fn parse_flags(ctx: &ParserCtx<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, flags) = separated_list1(
            delimited(multispace0, char('|'), multispace0),
            alt((Self::parse_builtin_type, Self::parse_flag, |i| {
                Self::parse_flags_variable(ctx, i)
            })),
        )(input)?;

        Ok((input, PatternArg::Flags(PatternFlagSet::new(flags))))
    }

    /// Parses built-in type constraints.
    ///
    /// # Supported Types
    ///
    /// - `<INTEGER>` - Matches any integer value
    /// - `<PATH>` - Matches file path strings
    ///
    /// # Format
    ///
    /// ```text
    /// <INTEGER>
    /// <PATH>
    /// ```
    fn parse_builtin_type(input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('<')(input)?;
        let (input, type_name) = identifier(input)?;
        let (input, _) = char('>')(input)?;

        match type_name {
            "INTEGER" => Ok((input, PatternArg::Integer)),
            "PATH" => Ok((input, PatternArg::Path)),
            _ => Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            ))),
        }
    }

    /// Parses a flags variable reference.
    ///
    /// # Format
    ///
    /// ```text
    /// <variable_name>
    /// ```
    ///
    /// The variable must be defined earlier in the SCML file using a
    /// flags definition statement.
    fn parse_flags_variable(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('<')(input)?;
        let (input, var_name) = identifier(input)?;
        let (input, _) = char('>')(input)?;

        if ctx.get_flags_id(var_name).is_none() {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }

        Ok((
            input,
            PatternArg::FlagsVariable(ctx.get_flags_id(var_name).unwrap()),
        ))
    }

    /// Parses a struct variable reference.
    ///
    /// # Format
    ///
    /// ```text
    /// <variable_name>
    /// ```
    ///
    /// The variable must be defined earlier in the SCML file using a
    /// struct definition statement.
    fn parse_struct_variable(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('<')(input)?;
        let (input, var_name) = identifier(input)?;
        let (input, _) = char('>')(input)?;

        if ctx.get_struct_id(var_name).is_none() {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }

        Ok((
            input,
            PatternArg::StructVariable(ctx.get_struct_id(var_name).unwrap()),
        ))
    }

    /// Parses a single flag identifier.
    ///
    /// # Format
    ///
    /// ```text
    /// FLAG_NAME
    /// O_RDONLY
    /// SA_NOCLDSTOP
    /// ```
    fn parse_flag(input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, flag_name) = identifier(input)?;
        Ok((input, PatternArg::Flag(flag_name)))
    }

    /// Returns whether this pattern accepts additional unspecified arguments.
    ///
    /// If `true`, syscalls with more arguments than specified in the pattern
    /// can still match. If `false`, the argument count must match exactly.
    pub fn wildcard(&self) -> bool {
        self.wildcard
    }

    /// Parses a variable definition (flags or struct).
    ///
    /// This method attempts to parse either a flags definition or struct definition
    /// and updates the parser context accordingly.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Mutable parser context to update with the definition
    /// * `input` - SCML source string to parse
    ///
    /// # Returns
    ///
    /// `IResult` with unit type on success, error on parse failure.
    pub fn parse_definition(ctx: &mut ParserCtx<'a>, input: &'a str) -> IResult<&'a str, ()> {
        if let Ok((input, (name, flags))) = Self::parse_flags_definition(ctx, input) {
            ctx.insert_flags_variable(name, flags);
            return Ok((input, ()));
        }

        if let Ok((input, (name, struct_def))) = Self::parse_struct_definition(ctx, input) {
            ctx.insert_struct_variable(name, struct_def);
            return Ok((input, ()));
        }

        ctx.set_last_struct(None);

        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }

    /// Parses a flags variable definition.
    ///
    /// # Format
    ///
    /// ```text
    /// variable_name = FLAG1 | FLAG2 | FLAG3;
    /// ```
    ///
    /// # Arguments
    ///
    /// * `ctx` - Parser context with variable definitions
    /// * `input` - SCML source string to parse
    ///
    /// # Returns
    ///
    /// `IResult` containing remaining input and parsed (name, PatternArg) tuple on success.
    fn parse_flags_definition(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, (&'a str, PatternArg<'a>)> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('=')(input)?;
        let (input, flags) = Self::parse_flags(ctx, input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(';')(input)?;

        Ok((input, (name, flags)))
    }

    /// Parses a struct variable definition.
    ///
    /// # Format
    ///
    /// ```text
    /// variable_name = struct {
    ///     field1: TYPE1,
    ///     field2: TYPE2,
    ///     ...
    /// };
    /// ```
    /// # Arguments
    ///
    /// * `ctx` - Parser context with variable definitions
    /// * `input` - SCML source string to parse
    ///
    /// # Returns
    ///
    /// `IResult` containing remaining input and parsed (name, PatternArg) tuple on success.
    fn parse_struct_definition(
        ctx: &ParserCtx<'a>,
        input: &'a str,
    ) -> IResult<&'a str, (&'a str, PatternArg<'a>)> {
        let (input, _) = multispace0(input)?;
        let (input, _) = tag("struct")(input)?;
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('=')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, struct_body) = Self::parse_struct(ctx, input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(';')(input)?;

        Ok((input, (name, struct_body)))
    }
}

/// Container for organizing multiple patterns by syscall name.
///
/// This structure stores all parsed patterns grouped by syscall name,
/// along with the parser context containing variable definitions.
#[derive(Debug, Clone, PartialEq)]
pub struct Patterns<'a> {
    /// Map from syscall names to their associated patterns.
    patterns: HashMap<&'a str, Vec<Pattern<'a>>>,

    /// Parser context with variable definitions.
    ctx: ParserCtx<'a>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParserCtx<'a> {
    /// Map of flags variable IDs to their pattern definitions.
    flags_variables: HashMap<&'a str, PatternArg<'a>>,

    /// Map of struct variable IDs to their pattern definitions.
    struct_variables: HashMap<&'a str, PatternArg<'a>>,

    /// Map of multiple struct variable IDs to their pattern definitions.
    multiple_struct_variables: HashMap<&'a str, PatternArg<'a>>,

    /// Map of named bitflags to their internal IDs.
    named_bitflags: HashMap<&'a str, &'a str>,

    /// Map of named structs to their internal IDs.
    named_structs: HashMap<&'a str, &'a str>,

    /// Last parsed struct definition.
    last_struct: Option<(&'a str, &'a str)>,
}

impl<'a> ParserCtx<'a> {
    /// Creates a new empty parser context.
    pub fn new() -> Self {
        Self {
            flags_variables: HashMap::new(),
            struct_variables: HashMap::new(),
            multiple_struct_variables: HashMap::new(),
            named_bitflags: HashMap::new(),
            named_structs: HashMap::new(),
            last_struct: None,
        }
    }

    /// Looks up a flags variable by its internal ID.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    ///
    /// # Returns
    ///
    /// `Some(&PatternArg)` if the variable exists, `None` otherwise.
    fn flags_lookup(&self, id: &str) -> Option<&PatternArg<'a>> {
        self.flags_variables.get(id)
    }

    /// Looks up a struct variable by its internal ID.
    ///
    /// Searches both regular and multiple struct variable maps.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    ///
    /// # Returns
    ///
    /// `Some(&PatternArg)` if the variable exists, `None` otherwise.
    fn struct_lookup(&self, id: &str) -> Option<&PatternArg<'a>> {
        if let Some(pattern) = self.struct_variables.get(id) {
            Some(pattern)
        } else {
            if let Some(pattern) = self.multiple_struct_variables.get(id) {
                Some(pattern)
            } else {
                None
            }
        }
    }

    /// Generates a unique variable ID.
    ///
    /// Creates a static string ID based on the total number of variables.
    ///
    /// # Returns
    ///
    /// A unique `&'static str` identifier.
    fn generate_variable_id(&self) -> &'static str {
        Box::leak(Box::new(format!(
            "var_{}",
            self.flags_variables.len()
                + self.struct_variables.len()
                + self.multiple_struct_variables.len()
                + 1
        )))
    }

    /// Inserts a named bitflag variable.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    /// * `id` - Internal variable ID
    fn insert_named_bitflag(&mut self, name: &'a str, id: &'a str) {
        self.named_bitflags.insert(name, id);
    }

    /// Inserts a named struct variable.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    /// * `id` - Internal variable ID
    fn insert_named_struct(&mut self, name: &'a str, id: &'a str) {
        self.named_structs.insert(name, id);
    }

    /// Adds a flags variable definition.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    /// * `flags` - Flags pattern definition
    fn add_flags_variable(&mut self, id: &'a str, flags: PatternArg<'a>) {
        self.flags_variables.insert(id, flags);
    }

    /// Adds a struct variable definition.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    /// * `struct_def` - Struct pattern definition
    fn add_struct_variable(&mut self, id: &'a str, struct_def: PatternArg<'a>) {
        self.struct_variables.insert(id, struct_def);
    }

    /// Adds an alternative struct definition to a multiple struct variable.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    /// * `struct_def` - Struct pattern definition
    fn add_multiple_struct_variable(&mut self, id: &'a str, struct_def: PatternArg<'a>) {
        if self.multiple_struct_variables.contains_key(id) {
            self.append_to_multiple_struct(id, struct_def);
        } else {
            self.convert_struct_to_multiple(id);
            self.append_to_multiple_struct(id, struct_def);
        }
    }

    /// Converts an existing struct variable to a multiple struct variable.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    ///
    /// # Panics
    ///
    /// Panics if the struct variable does not exist.
    fn convert_struct_to_multiple(&mut self, id: &'a str) {
        if let Some(existing_struct) = self.struct_variables.remove(id) {
            let multiple_struct =
                PatternArg::MultipleStruct(PatternMultipleStruct::new(vec![existing_struct]));
            self.multiple_struct_variables.insert(id, multiple_struct);
        } else {
            panic!("Struct variable should exist for conversion");
        }
    }

    /// Appends a struct definition to an existing multiple struct variable.
    ///
    /// # Arguments
    ///
    /// * `id` - Internal variable ID
    /// * `struct_def` - Struct pattern definition
    ///
    /// # Panics
    ///
    /// Panics if the multiple struct variable does not exist or is of incorrect type.
    fn append_to_multiple_struct(&mut self, id: &str, struct_def: PatternArg<'a>) {
        let multiple_struct = self
            .multiple_struct_variables
            .get_mut(id)
            .expect("Multiple struct variable should exist");

        if let PatternArg::MultipleStruct(multi_struct) = multiple_struct {
            multi_struct.0.push(struct_def);
        } else {
            panic!("Expected MultipleStruct variant");
        }
    }

    /// Updates the last defined struct for consecutive definition tracking.
    ///
    /// # Arguments
    ///
    /// * `value` - Optional tuple containing the last struct name and ID
    fn set_last_struct(&mut self, value: Option<(&'a str, &'a str)>) {
        self.last_struct = value;
    }

    /// Retrieves the internal ID for a flags variable by name.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    ///
    /// # Returns
    ///
    /// `Some(&'a str)` if the variable exists, `None` otherwise.
    fn get_flags_id(&self, name: &str) -> Option<&'a str> {
        self.named_bitflags.get(name).map(|id| *id)
    }

    /// Retrieves the internal ID for a struct variable by name.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    ///
    /// # Returns
    ///
    /// `Some(&'a str)` if the variable exists, `None` otherwise.
    fn get_struct_id(&self, name: &str) -> Option<&'a str> {
        self.named_structs.get(name).map(|id| *id)
    }

    /// Inserts a flags variable definition.
    ///
    /// Creates a new variable ID and registers the flags pattern.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    /// * `flags` - Flags pattern definition
    fn insert_flags_variable(&mut self, name: &'a str, flags: PatternArg<'a>) {
        let id = self.generate_variable_id();
        self.insert_named_bitflag(name, id);
        self.add_flags_variable(id, flags);
        self.set_last_struct(None);
    }

    /// Inserts a struct variable definition.
    ///
    /// If the struct name matches the previously defined struct name, treats it
    /// as an alternative definition and creates or appends to a multiple struct.
    /// Otherwise, creates a new struct variable.
    ///
    /// # Arguments
    ///
    /// * `name` - User-facing variable name
    /// * `struct_def` - Struct pattern definition
    fn insert_struct_variable(&mut self, name: &'a str, struct_def: PatternArg<'a>) {
        if let Some((last_struct_name, last_struct_id)) = &self.last_struct {
            // Continuous definition - append as alternative
            if &name == last_struct_name {
                self.add_multiple_struct_variable(last_struct_id, struct_def);
                return;
            }
        }

        // New struct definition
        let id = self.generate_variable_id();
        self.insert_named_struct(name, id);
        self.add_struct_variable(id, struct_def);
        self.set_last_struct(Some((name, id)));
    }
}

/// Parses identifiers from SCML source.
///
/// Accepts both numeric identifiers (for literal values) and alphanumeric
/// identifiers starting with a letter or underscore (for names and flags).
///
/// # Accepted Formats
///
/// - Pure digits: `42`, `0x10`, `100`
/// - Alphanumeric: `variable_name`, `FLAG_NAME`, `_private`
/// - Mixed: `var123`, `FLAG_1`
///
/// # Arguments
///
/// * `input` - Source string to parse
///
/// # Returns
///
/// `IResult` containing the parsed identifier.
fn identifier(input: &str) -> IResult<&str, &str> {
    alt((
        nom::character::complete::digit1,
        recognize(pair(
            alt((nom::character::complete::alpha1, tag("_"))),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
    ))(input)
}

impl<'a> Patterns<'a> {
    /// Creates a new Patterns container.
    ///
    /// # Arguments
    ///
    /// * `patterns` - Map from syscall names to their patterns
    /// * `ctx` - Parser context with variable definitions
    pub fn new(patterns: HashMap<&'a str, Vec<Pattern<'a>>>, ctx: ParserCtx<'a>) -> Self {
        Self { patterns, ctx }
    }

    /// Parses an SCML file and returns a `Patterns` container.
    ///
    /// Reads the file content and parses all pattern and variable definitions.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the SCML file
    ///
    /// # Returns
    ///
    /// Returns `Ok(Patterns)` on success, or `ScmlParseError` on failure.
    ///
    /// # Errors
    ///
    /// - `IoError` - File reading failed
    /// - `ParseError` - SCML syntax errors
    /// - `IncompleteStatement` - Missing semicolon before EOF
    pub fn from_scml_file(path: &str) -> Result<Self, ScmlParseError> {
        let content = fs::read_to_string(path).map_err(|e| {
            ScmlParseError::IoError(format!("Failed to read file '{}': {}", path, e))
        })?;

        Self::from_scml(&content)
    }

    /// Parses SCML content from a string.
    ///
    /// Processes all statements in the content, separating variable definitions
    /// from pattern definitions. Variable definitions are processed first to
    /// build the context, then patterns are parsed using that context.
    ///
    /// # Arguments
    ///
    /// * `content` - SCML file content as a string
    ///
    /// # Returns
    ///
    /// Returns `Ok(Patterns)` on success, or `ScmlParseError` on failure.
    ///
    /// # Errors
    ///
    /// - `ParseError` - SCML syntax errors with details
    /// - `IncompleteStatement` - Missing semicolon before EOF
    pub fn from_scml(content: &str) -> Result<Self, ScmlParseError> {
        let stmt_iterator = StatementIterator::new(content);
        let mut patterns: HashMap<&str, Vec<Pattern>> = HashMap::new();
        let mut ctx = ParserCtx::new();
        let mut errors = Vec::new();

        for stmt in stmt_iterator {
            let statement = stmt?;

            if Pattern::parse_definition(&mut ctx, statement).is_ok() {
                // Successfully parsed a definition, continue to next statement
                continue;
            }

            match Pattern::parse(&ctx, statement) {
                Ok((remaining, pattern)) => {
                    if !remaining.trim().is_empty() {
                        errors.push(format!(
                            "Warning: Unparsed input remaining in statement '{}': '{}'",
                            statement, remaining
                        ));
                    }
                    patterns.entry(pattern.name()).or_default().push(pattern);
                }
                Err(err) => {
                    errors.push(format!("Error parsing statement '{}': {}", statement, err));
                    break;
                }
            }
        }

        // If there are any parsing errors, return them
        if !errors.is_empty() {
            return Err(ScmlParseError::ParseError(errors.join("\n")));
        }

        Ok(Self::new(patterns, ctx))
    }

    /// Retrieves all patterns for a specific syscall name.
    ///
    /// # Arguments
    ///
    /// * `name` - The syscall name to look up
    ///
    /// # Returns
    ///
    /// `Some(&Vec<Pattern>)` if patterns exist for the syscall, `None` otherwise.
    pub fn get(&self, name: &str) -> Option<&Vec<Pattern>> {
        self.patterns.get(name)
    }

    /// Returns the total number of unique syscall patterns stored.
    ///
    /// This counts the number of distinct syscall names that have patterns,
    /// not the total number of pattern variations.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Returns a reference to the parser context.
    ///
    /// The context contains all variable definitions used during parsing.
    pub fn ctx(&self) -> &ParserCtx {
        &self.ctx
    }
}

/// Iterator that yields complete statements from SCML content.
///
/// This iterator handles:
/// - Multi-line statements (statements can span multiple lines)
/// - Comment removal (lines starting with `//`)
/// - Empty line filtering
/// - Statement completion detection (ending with `;`)
struct StatementIterator<'a> {
    /// Line iterator for the SCML content.
    lines: std::str::Lines<'a>,

    /// Buffer for accumulating multi-line statements.
    current_statement: String,
}

impl<'a> StatementIterator<'a> {
    /// Creates a new statement iterator for the given SCML content.
    fn new(content: &'a str) -> Self {
        Self {
            lines: content.lines(),
            current_statement: String::new(),
        }
    }
}

impl<'a> Iterator for StatementIterator<'a> {
    type Item = Result<&'static str, ScmlParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.lines.next() {
                Some(line) => {
                    let line = line.trim();
                    // Skip comments and empty lines
                    if line.starts_with("//") || line.is_empty() {
                        continue;
                    }

                    self.current_statement.push_str(line.trim_end());
                    self.current_statement.push(' ');

                    // Check if statement is complete
                    if self.current_statement.trim().ends_with(';') {
                        let statement = self.current_statement.trim().to_string();
                        self.current_statement.clear();
                        // Leak the string to get a 'static lifetime reference
                        let leaked: &'static str = Box::leak(statement.into_boxed_str());
                        return Some(Ok(leaked));
                    }
                }
                None => {
                    // End of input
                    if !self.current_statement.trim().is_empty() {
                        let incomplete = self.current_statement.trim().to_string();
                        self.current_statement.clear();
                        return Some(Err(ScmlParseError::IncompleteStatement(incomplete)));
                    }
                    return None;
                }
            }
        }
    }
}
