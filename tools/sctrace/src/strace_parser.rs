// SPDX-License-Identifier: MPL-2.0

//! Parser for strace output.
//!
//! This module provides functionality to parse system call traces from strace output.
//! It handles various strace line formats including regular syscalls, blocked/resumed
//! syscalls in multi-threaded scenarios, signal lines, and exit status lines.
//!
//! # Example
//!
//! ```text
//! use strace_parser::Syscall;
//!
//! let line = "123 openat(AT_FDCWD</home/user>, \"file.txt\", O_RDONLY) = 3";
//! match Syscall::parse(line) {
//!     Ok(syscall) => println!("Parsed syscall: {}", syscall.name()),
//!     Err(e) => eprintln!("Parse error: {}", e),
//! }
//! ```

use std::{cell::RefCell, collections::HashMap, error::Error, fmt};

use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, opt, peek, recognize, rest, value},
    multi::{separated_list0, separated_list1},
    sequence::{delimited, preceded, separated_pair, terminated, tuple},
};

thread_local! {
    /// Thread-local storage for blocked syscalls in multi-threaded traces.
    ///
    /// When parsing multi-threaded strace output, syscalls may be split across
    /// multiple lines with `<unfinished ...>` and `<... resumed>` markers.
    /// This storage keeps track of blocked syscalls by PID until they are resumed.
    static BLOCKED_SYSCALL: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// Custom error type for strace parsing.
///
/// This enum represents various error conditions that can occur during
/// strace output parsing, including special cases like signal lines and
/// exit status lines that should be skipped.
#[derive(Debug, Clone, PartialEq)]
pub enum StraceParseError {
    /// A syscall line was blocked and skipped.
    ///
    /// This occurs when encountering a line with `<unfinished ...>` marker
    /// in multi-threaded traces. The partial syscall is saved for later
    /// reconstruction when the corresponding `<... resumed>` line is found.
    BlockedLine,

    /// Signal line was encountered and skipped.
    ///
    /// Signal lines have the format `--- SIGNAME {...} ---` and are not
    /// relevant for syscall tracing purposes.
    SignalLine,

    /// Exit status line was encountered and skipped.
    ///
    /// Exit lines have the format `+++ exited with N +++` and indicate
    /// process termination rather than syscall activity.
    ExitLine,

    /// Nom parsing error with context.
    ///
    /// Contains the error message and the input string that failed to parse.
    ParseError {
        /// Description of the parsing error
        message: String,
        /// The input string that failed to parse
        input: String,
    },

    /// Type validation error.
    ///
    /// Occurs when an argument type constraint is violated, such as
    /// invalid flag set members.
    TypeError(String),
}

impl fmt::Display for StraceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StraceParseError::BlockedLine => write!(f, "Blocked syscall line, skipped"),
            StraceParseError::SignalLine => write!(f, "Signal line, skipped"),
            StraceParseError::ExitLine => write!(f, "Exit status line, skipped"),
            StraceParseError::ParseError { message, input } => {
                write!(f, "{} (input: {})", message, input)
            }
            StraceParseError::TypeError(msg) => write!(f, "Type error: {}", msg),
        }
    }
}

impl Error for StraceParseError {}

/// Represents a system call captured from strace output.
///
/// This structure contains all the information extracted from a single
/// strace line, including process ID, syscall name, arguments, return value,
/// and the original unparsed line for reference.
///
/// # Example
///
/// For an strace line like:
/// ```text
/// 123 openat(AT_FDCWD</home/user>, "file.txt", O_RDONLY) = 3
/// ```
///
/// The parsed `Syscall` would have:
/// - `pid`: 123
/// - `name`: "openat"
/// - `args`: [FdPath("/home/user"), String("file.txt"), Flag("O_RDONLY")]
/// - `return_value`: "3"
#[derive(Debug, PartialEq, Clone)]
pub struct Syscall<'a> {
    /// Process ID that made the syscall.
    pid: u32,

    /// Name of the system call.
    name: &'a str,

    /// Arguments passed to the syscall.
    args: Vec<SyscallArg<'a>>,

    /// Return value of the syscall.
    ///
    /// Since the return value format varies, this field stores the entire
    /// string after `=`, which may include error codes and descriptions,
    /// such as `-1 ENOENT (No such file or directory)`.
    return_value: &'a str,

    /// The original input line from strace.
    ///
    /// Preserved for debugging and reference purposes.
    original_line: &'a str,
}

/// Represents different types of syscall arguments.
///
/// This enum covers all possible argument types that can appear in
/// strace output, from simple integers and strings to complex
/// structures and arrays.
#[derive(Debug, PartialEq, Clone)]
pub enum SyscallArg<'a> {
    /// Numeric values (integers, hex, etc.).
    ///
    /// Includes decimal numbers (e.g., `123`, `-456`), hexadecimal numbers
    /// (e.g., `0x1A2B`), and numbers with shift/multiply operations
    /// (e.g., `4096<<2`, `10*8`).
    Integer(&'a str),

    /// Quoted string literals.
    ///
    /// String arguments enclosed in double quotes, potentially with
    /// escape sequences. May be prefixed with `@` or suffixed with `...`
    /// to indicate truncation.
    String(&'a str),

    /// Unquoted string flags or identifiers.
    ///
    /// Symbolic constants like `O_RDONLY`, `PROT_READ`, or other
    /// named flags and enumerations.
    Flag(&'a str),

    /// File descriptor with associated path.
    ///
    /// Represents file descriptors as shown by strace's `-yy` option,
    /// which displays the path associated with each fd in angle brackets,
    /// e.g., `3</path/to/file>` or `AT_FDCWD</current/dir>`.
    FdPath(&'a str),

    /// Combination of flags and/or numbers separated by '|' or " or ".
    ///
    /// Represents bitwise OR combinations of flags, such as
    /// `O_RDWR|O_CREAT` or `PROT_READ or PROT_WRITE`.
    Flags(SyscallFlagSet<'a>),

    /// Key-value pairs enclosed in {}.
    ///
    /// Represents C struct arguments, parsed as maps from field names
    /// to their values, e.g., `{st_mode=S_IFREG|0644, st_size=1024}``.
    Struct(SyscallStruct<'a>),

    /// List of SyscallArgs enclosed in [].
    ///
    /// Represents array arguments, such as file descriptor sets or
    /// string arrays like `["arg1", "arg2", "arg3"]`.
    Array(SyscallArray<'a>),

    /// Arguments that can be ignored for matching purposes.
    ///
    /// Used for arguments that are not relevant for analysis, such as
    /// masks (`~[...]`), empty arguments, or unparsable content.
    Ignored,
}

/// Wrapper for arrays of syscall arguments.
///
/// Contains a vector of syscall arguments that can be of different types.
/// This allows parsing of heterogeneous arrays from strace output.
///
/// # Example
///
/// Arrays can contain mixed types:
/// ```text
/// [1, "string", O_RDONLY]
/// ["arg1", 123, {field=value}]
/// ```
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallArray<'a>(Vec<SyscallArg<'a>>);

/// Wrapper for flag sets with restricted element types.
///
/// Elements can only be Flag or Integer types, representing symbolic
/// flags or numeric bit masks that are combined with bitwise OR.
///
/// # Example
///
/// Valid: `O_RDWR|O_CREAT|0x200`
/// Invalid: `O_RDONLY|"string"` (strings not allowed in flag sets)
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallFlagSet<'a>(Vec<SyscallArg<'a>>);

/// Wrapper for structured data represented as key-value pairs.
///
/// Represents C struct arguments as hash maps, where keys are field
/// names and values are the corresponding argument values.
///
/// # Example
///
/// For `{st_mode=S_IFREG|0644, st_size=1024}`, the fields would be:
/// - "st_mode" -> Flags(...)
/// - "st_size" -> Integer("1024")
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallStruct<'a>(HashMap<&'a str, SyscallArg<'a>>);

impl<'a> Syscall<'a> {
    /// Creates a new Syscall instance.
    ///
    /// # Arguments
    ///
    /// * `pid` - Process ID that made the syscall
    /// * `name` - Name of the system call
    /// * `args` - Vector of parsed arguments
    /// * `return_value` - Return value string from strace output
    /// * `original_line` - The original unparsed strace line
    pub fn new(
        pid: u32,
        name: &'a str,
        args: Vec<SyscallArg<'a>>,
        return_value: &'a str,
        original_line: &'a str,
    ) -> Self {
        Self {
            pid,
            name,
            args,
            return_value,
            original_line,
        }
    }

    /// Fetches and preprocesses a strace line before parsing.
    ///
    /// This method handles special line types that need preprocessing:
    /// - Signal lines: Returns `Err(StraceParseError::SignalLine)`
    /// - Exit lines: Returns `Err(StraceParseError::ExitLine)`
    /// - Blocked lines: Saves partial syscall, returns `Err(StraceParseError::BlockedLine)`
    /// - Resumed lines: Reconstructs complete syscall from saved partial
    /// - Normal lines: Returns unchanged
    ///
    /// # Arguments
    ///
    /// * `line` - A single line of strace output
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` containing the preprocessed line ready for parsing,
    /// or an appropriate `StraceParseError` if the line should be skipped.
    ///
    /// # Example
    ///
    /// ```text
    /// // Normal line passes through
    /// let line = "openat(AT_FDCWD, \"file.txt\", O_RDONLY) = 3".to_string();
    /// let result = Syscall::fetch(line)?;
    ///
    /// // Blocked line saves state
    /// let blocked = "123 openat(AT_FDCWD, \"file.txt\" <unfinished ...>".to_string();
    /// assert!(matches!(Syscall::fetch(blocked), Err(StraceParseError::BlockedLine)));
    ///
    /// // Resumed line reconstructs
    /// let resumed = "123 <... openat resumed>, O_RDONLY) = 3".to_string();
    /// let reconstructed = Syscall::fetch(resumed)?;
    /// assert_eq!(reconstructed, "123 openat(AT_FDCWD, \"file.txt\", O_RDONLY) = 3");
    /// ```
    pub fn fetch(line: String) -> Result<String, StraceParseError> {
        let trimmed = line.as_str().trim();

        // Skip signal lines
        if let Ok(_) = Self::parse_signal_line(trimmed) {
            return Err(StraceParseError::SignalLine);
        }

        // Skip exit status lines
        if let Ok(_) = Self::parse_exit_line(trimmed) {
            return Err(StraceParseError::ExitLine);
        }

        // Save blocked syscalls for later reconstruction
        if let Ok((_, (pid, str))) = Self::parse_multithread_blocked(trimmed) {
            BLOCKED_SYSCALL.with(|blocked| {
                blocked.borrow_mut().insert(pid, str);
            });
            return Err(StraceParseError::BlockedLine);
        }

        if let Ok((_, (pid, resumed))) = Self::parse_multithread_resumed(trimmed) {
            let blocked_call =
                BLOCKED_SYSCALL.with(|blocked| blocked.borrow().get(&pid).cloned().unwrap());
            let reconstructed = format!("{} {}{}", pid, blocked_call, resumed);
            return Ok(reconstructed);
        }

        Ok(line)
    }

    /// Parses a single line of strace output into a Syscall.
    ///
    /// This is the main entry point for parsing. It expects a preprocessed
    /// line from `fetch()` and parses it into a structured `Syscall` object.
    ///
    /// # Arguments
    ///
    /// * `input` - A single preprocessed line of strace output
    ///
    /// # Returns
    ///
    /// Returns `Ok(Syscall)` on successful parse, or a `StraceParseError`
    /// if the line cannot be parsed.
    ///
    /// # Example
    ///
    /// ```text
    /// let line = "openat(AT_FDCWD, \"file.txt\", O_RDONLY) = 3";
    /// let syscall = Syscall::parse(line)?;
    /// assert_eq!(syscall.name(), "openat");
    /// assert_eq!(syscall.args().len(), 3);
    /// ```
    pub fn parse(input: &'a str) -> Result<Self, StraceParseError> {
        let trimmed = input.trim();

        let syscall = Self::parse_syscall_line(trimmed)
            .map(|(_, syscall)| syscall)
            .map_err(|e| StraceParseError::ParseError {
                message: e.to_string(),
                input: trimmed.to_string(),
            })?;

        let syscall = Self::handle_special_cases(syscall);

        Ok(syscall)
    }

    /// Handles special cases for certain syscalls whose strace output is non-standard.
    ///
    /// Some syscalls have arguments that strace omits or represents differently.
    /// This method adjusts the parsed arguments to match the actual syscall signature.
    ///
    /// # Current Special Cases
    ///
    /// - `clone`: strace removes the first and fourth arguments. This method
    ///   inserts `Ignored` placeholders at positions 0 and 3 to maintain
    ///   correct argument indexing.
    ///
    /// # Arguments
    ///
    /// * `syscall` - The parsed syscall to potentially modify
    ///
    /// # Returns
    ///
    /// Returns the syscall, possibly with modified arguments.
    fn handle_special_cases(mut syscall: Syscall) -> Syscall {
        match syscall.name {
            // strace remove the first and fourth arguments for `clone`, just insert
            // ignored args.
            "clone" => {
                syscall.args.insert(0, SyscallArg::Ignored);
                syscall.args.insert(3, SyscallArg::Ignored);
            }
            _ => {}
        }

        syscall
    }

    /// Parses a complete syscall line including optional PID prefix.
    ///
    /// # Format
    ///
    /// ```text
    /// [PID] syscall_name(args...) = return_value
    /// ```
    ///
    /// The PID is optional and defaults to 0 if not present.
    fn parse_syscall_line(input: &str) -> IResult<&str, Syscall> {
        map(
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                Self::parse_syscall_content,
            )),
            |(pid, (name, args, return_value))| {
                Syscall::new(pid.unwrap_or(0), name, args, return_value, input)
            },
        )(input)
    }

    /// Parses the main content of a syscall: name, arguments, and return value.
    ///
    /// # Format
    ///
    /// ```text
    /// syscall_name(args...) = return_value
    /// ```
    fn parse_syscall_content(input: &str) -> IResult<&str, (&str, Vec<SyscallArg>, &str)> {
        tuple((Self::parse_name, Self::parse_args, Self::parse_return_value))(input)
    }

    /// Parses the syscall name.
    ///
    /// The name consists of alphanumeric characters and underscores.
    /// Leading and trailing whitespace is ignored.
    ///
    /// # Examples
    ///
    /// - `openat`
    /// - `mmap`
    /// - `rt_sigaction`
    fn parse_name(input: &str) -> IResult<&str, &str> {
        delimited(
            space0,
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
            space0,
        )(input)
    }

    /// Parses the arguments of the syscall.
    ///
    /// The arguments are enclosed in parentheses and separated by commas.
    /// Leading and trailing whitespace around each argument is ignored.
    ///
    /// # Format
    ///
    /// ```text
    /// (arg1, arg2, arg3, ...)
    /// ```
    fn parse_args(input: &str) -> IResult<&str, Vec<SyscallArg>> {
        delimited(
            char('('),
            separated_list0(char(','), delimited(space0, Self::parse_arg, space0)),
            char(')'),
        )(input)
    }

    /// Parses a single syscall argument.
    ///
    /// An argument can be one of many types. The parser tries each type
    /// in order using the `alt` combinator. This method also handles:
    /// - Optional parameter names (e.g., `flags=O_RDONLY`)
    /// - Trailing comments (e.g., `/* comment */`)
    /// - Arrow mappings (e.g., `=> other_value`)
    ///
    /// # Supported Types
    ///
    /// - None (empty argument)
    /// - FdPath (file descriptor with path)
    /// - Struct (key-value pairs in braces)
    /// - Array (elements in brackets)
    /// - Quoted strings
    /// - Masks (bitwise complements)
    /// - Hexadecimal numbers
    /// - Decimal numbers
    /// - Flag combinations
    /// - Unquoted flags
    fn parse_arg(input: &str) -> IResult<&str, SyscallArg> {
        let (input, arg) = preceded(
            // Skip parameter name and equals sign if present
            opt(terminated(
                take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
                char('='),
            )),
            alt((
                Self::parse_none,
                Self::parse_fd_path,
                Self::parse_struct,
                Self::parse_array,
                Self::parse_quoted_string,
                Self::parse_mask,
                Self::parse_hex,
                Self::parse_number,
                Self::parse_flags,
                Self::parse_unquoted_flag,
            )),
        )(input)?;

        // Skip optional comment
        let (input, _) = opt(delimited(
            delimited(space0, tag("/*"), space0),
            take_until("*/"),
            tag("*/"),
        ))(input)?;

        let (input, _) = opt(preceded(
            delimited(space0, tag("=>"), space0),
            Self::parse_arg,
        ))(input)?;

        Ok((input, arg))
    }

    /// Parses the fd path argument.
    ///
    /// With strace's `-yy` option, file descriptors are annotated with their
    /// associated paths or sockets in angle brackets.
    ///
    /// # Format
    ///
    /// ```text
    /// fd</path/to/file>
    /// AT_FDCWD</current/directory>
    /// 3</home/user/file.txt>
    /// 5<TCP:[271908242]>
    /// ```
    ///
    /// The parsed path is returned in the `SyscallArg::FdPath` variant.
    fn parse_fd_path(input: &str) -> IResult<&str, SyscallArg> {
        map(
            tuple((
                alt((tag("AT_FDCWD"), take_while1(|c: char| c.is_ascii_digit()))),
                Self::parse_angle_bracket_content,
            )),
            |(_, path)| SyscallArg::FdPath(path),
        )(input)
    }

    /// Parses content within angle brackets, handling nested brackets and arrows.
    ///
    /// This handles various complex cases:
    /// ```text
    /// <anon_inode:[eventfd]>
    /// <socket:[12345]>
    /// <UDP:[127.0.0.1:59926->127.0.0.1:1025]>
    /// ```
    fn parse_angle_bracket_content(input: &str) -> IResult<&str, &str> {
        let (input, _) = char::<&str, nom::error::Error<&str>>('<')(input)?;

        let mut depth = 1;
        let mut end_pos = 0;
        let mut chars = input.char_indices().peekable();

        while let Some((i, ch)) = chars.next() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    // Check if this is part of '->' arrow
                    // Look back to see if previous char was '-'
                    if i > 0 && input.as_bytes().get(i - 1) == Some(&b'-') {
                        // This '>' is part of '->', not a closing bracket
                        continue;
                    }

                    depth -= 1;
                    if depth == 0 {
                        end_pos = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }

        let content = &input[..end_pos];
        let remaining = &input[end_pos + 1..];

        Ok((remaining, content))
    }

    /// Parses a quoted string argument.
    ///
    /// Handles escape sequences within the string. Supports optional
    /// `@` prefix (for abstract socket names) and `...` suffix
    /// (indicating truncation by strace).
    ///
    /// # Format
    ///
    /// ```text
    /// "normal string"
    /// @"abstract socket"
    /// "truncated string"...
    /// "string with \"escapes\""
    /// ```
    ///
    /// The parsed string is returned in the `SyscallArg::String` variant.
    fn parse_quoted_string(input: &str) -> IResult<&str, SyscallArg> {
        map(
            tuple((
                opt(char('@')), // Optional @ prefix
                delimited(char('"'), Self::take_until_unescaped_quote, char('"')),
                opt(tag("...")), // Optional ... suffix
            )),
            |(_, content, _)| SyscallArg::String(content),
        )(input)
    }

    /// Helper to take characters until an unescaped quote is found.
    ///
    /// Properly handles backslash escapes, including escaped backslashes.
    fn take_until_unescaped_quote(input: &str) -> IResult<&str, &str> {
        let mut chars = input.char_indices();
        let mut last_was_escape = false;

        while let Some((i, ch)) = chars.next() {
            if ch == '"' && !last_was_escape {
                return Ok((&input[i..], &input[..i]));
            }
            last_was_escape = ch == '\\' && !last_was_escape;
        }

        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }

    /// Parses a hexadecimal number argument.
    ///
    /// # Format
    ///
    /// ```text
    /// 0x1A2B3C
    /// 0xDEADBEEF
    /// 0x0
    /// ```
    ///
    /// The number is prefixed by `0x` and consists of hexadecimal digits.
    /// The parsed number is returned as a string in the `SyscallArg::Integer` variant.
    fn parse_hex(input: &str) -> IResult<&str, SyscallArg> {
        map(
            recognize(preceded(
                tag("0x"),
                take_while1(|c: char| c.is_ascii_hexdigit()),
            )),
            |s: &str| SyscallArg::Integer(s),
        )(input)
    }

    /// Parses a decimal number argument.
    ///
    /// Supports negative numbers and arithmetic operations like
    /// left shift and multiplication.
    ///
    /// # Format
    ///
    /// ```text
    /// 123
    /// -456
    /// 4096<<2
    /// 10*8
    /// -1
    /// ```
    ///
    /// The parsed number is returned as a string in the `SyscallArg::Integer` variant.
    fn parse_number(input: &str) -> IResult<&str, SyscallArg> {
        map(
            recognize(tuple((
                opt(char('-')),
                digit1,
                opt(tuple((alt((tag("<<"), tag("*"))), opt(char('-')), digit1))),
            ))),
            |s: &str| SyscallArg::Integer(s),
        )(input)
    }

    /// Parses a set of flags combined with bitwise OR.
    ///
    /// Flags can be separated by `|` or ` or ` (literal text " or ").
    /// Each flag can be a hex number, decimal number, or symbolic flag name.
    ///
    /// # Format
    ///
    /// ```text
    /// O_RDWR|O_CREAT
    /// SNDCTL_TMR_STOP or TCSETSW
    /// 0x1|0x2
    /// FLAG1|FLAG2|0x100
    /// ```
    ///
    /// The parsed flags are returned as a `SyscallArg::Flags` variant
    /// containing a `SyscallFlagSet`.
    fn parse_flags(input: &str) -> IResult<&str, SyscallArg> {
        map(
            separated_list1(
                alt((tag(" or "), tag("|"))),
                alt((
                    Self::parse_hex,
                    Self::parse_number,
                    Self::parse_unquoted_flag,
                )),
            ),
            |flags| SyscallArg::Flags(SyscallFlagSet::new(flags).unwrap()),
        )(input)
    }

    /// Parses an unquoted flag argument.
    ///
    /// Flags are symbolic constants that may optionally include:
    /// - Parenthetical parameters: `FLAG(param)`
    /// - Left shift operations: `FLAG<<2`
    /// - Both: `FLAG(param)<<2`
    ///
    /// # Format
    ///
    /// ```text
    /// O_RDONLY
    /// PROT_READ
    /// makedev(0x1, 0x3)
    /// FUTEX_OP_OR<<28
    /// ```
    ///
    /// The parsed flag is returned as a string in the `SyscallArg::Flag` variant.
    fn parse_unquoted_flag(input: &str) -> IResult<&str, SyscallArg> {
        map(
            recognize(tuple((
                take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '?'),
                opt(delimited(
                    char::<&str, nom::error::Error<&str>>('('),
                    take_until(")"),
                    char::<&str, nom::error::Error<&str>>(')'),
                )),
                opt(tuple((
                    tag("<<"),
                    take_while1(|c: char| c.is_ascii_digit()),
                ))),
            ))),
            |matched: &str| SyscallArg::Flag(matched),
        )(input)
    }

    /// Parses a mask argument.
    ///
    /// Masks represent bitwise complements and are typically used for
    /// signal sets. They have the format `~[flags]`.
    ///
    /// # Format
    ///
    /// ```text
    /// ~[SIGTERM SIGINT]
    /// ~[]
    /// ```
    ///
    /// The parsed mask is returned as `SyscallArg::Ignored` since the
    /// specific flags are not relevant for most analyses.
    fn parse_mask(input: &str) -> IResult<&str, SyscallArg> {
        value(
            SyscallArg::Ignored,
            delimited(tag("~["), take_until("]"), char(']')),
        )(input)
    }

    /// Parses a struct argument.
    ///
    /// Structs represent C struct arguments as key-value pairs enclosed
    /// in curly braces. Each value is recursively parsed as an argument.
    ///
    /// # Format
    ///
    /// ```text
    /// {st_mode=S_IFREG|0644, st_size=1024}
    /// {sa_handler=SIG_DFL, sa_mask=[], sa_flags=0}
    /// {tv_sec=10, tv_nsec=500000000}
    /// {...}  // Abbreviated by strace
    /// ```
    ///
    /// If the struct contains only `...`, it is treated as ignored.
    /// If parsing fails, the struct is also treated as ignored. E.g.,
    ///
    /// ```text
    /// {WIFEXITED(s) && WEXITSTATUS(s) == 0}
    /// ```
    ///
    /// The parsed struct is returned as a `SyscallArg::Struct` variant
    /// containing a `SyscallStruct`.
    fn parse_struct(input: &str) -> IResult<&str, SyscallArg> {
        alt((
            // Try to parse as key-value struct
            map(
                delimited(
                    char('{'),
                    separated_list0(
                        char(','),
                        preceded(
                            space0,
                            alt((
                                value(None, tag("...")),
                                map(
                                    separated_pair(
                                        take_while1(|c: char| {
                                            c.is_ascii_alphanumeric() || c == '_'
                                        }),
                                        char('='),
                                        Self::parse_arg,
                                    ),
                                    |(k, v)| Some((k, v)),
                                ),
                            )),
                        ),
                    ),
                    char('}'),
                ),
                |pairs| {
                    let fields: HashMap<&str, SyscallArg> =
                        pairs.into_iter().filter_map(|x| x).collect();

                    if fields.is_empty() {
                        SyscallArg::Ignored
                    } else {
                        SyscallArg::Struct(SyscallStruct::new(fields))
                    }
                },
            ),
            // Fallback: treat as ignored if it doesn't match struct pattern
            value(
                SyscallArg::Ignored,
                delimited(char('{'), take_until("}"), char('}')),
            ),
        ))(input)
    }

    /// Parses an array argument.
    ///
    /// Arrays consist of comma-separated arguments enclosed in square brackets.
    /// Each element is recursively parsed and can be of different types.
    ///
    /// # Format
    ///
    /// ```text
    /// [1, 2, 3, 4]
    /// ["arg1", "arg2", "arg3"]
    /// [O_RDONLY, "string", 123]  // Mixed types allowed
    /// []  // Empty array
    /// ```
    ///
    /// If the array cannot be parsed, it is treated as ignored.
    ///
    /// The parsed array is returned as a `SyscallArg::Array` variant
    /// containing a `SyscallArray`.
    fn parse_array(input: &str) -> IResult<&str, SyscallArg> {
        alt((
            |input| {
                let (remaining, elements) = delimited(
                    char('['),
                    separated_list0(char(','), preceded(space0, Self::parse_arg)),
                    char(']'),
                )(input)?;

                match SyscallArray::new(elements) {
                    Ok(array) => Ok((remaining, SyscallArg::Array(array))),
                    Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                        input,
                        nom::error::ErrorKind::Verify,
                    ))),
                }
            },
            value(
                SyscallArg::Ignored,
                delimited(char('['), take_until("]"), char(']')),
            ),
        ))(input)
    }

    /// Parses the return value of the syscall.
    ///
    /// The return value starts with `=` and continues to the end of the line.
    /// It may include error codes and descriptions.
    ///
    /// # Format
    ///
    /// ```text
    /// = 0
    /// = 3
    /// = -1 ENOENT (No such file or directory)
    /// = -1 EAGAIN (Resource temporarily unavailable)
    /// = ? ERESTARTSYS (To be restarted if SA_RESTART is set)
    /// ```
    ///
    /// The parsed return value is returned as a trimmed string.
    fn parse_return_value(input: &str) -> IResult<&str, &str> {
        preceded(
            tuple((space0, char('='), space0)),
            map(rest, |s: &str| s.trim()),
        )(input)
    }

    /// Parses a PID, which is a sequence of digits, and converts it to u32.
    ///
    /// PIDs appear at the beginning of strace lines when tracing multiple
    /// processes with the `-f` or `-ff` flags.
    fn parse_pid(input: &str) -> IResult<&str, u32> {
        map(digit1, |s: &str| s.parse::<u32>().unwrap())(input)
    }

    /// Parses a None argument, which is represented by an empty argument.
    ///
    /// This occurs when there are two consecutive commas or a comma
    /// immediately followed by a closing parenthesis.
    ///
    /// # Format
    ///
    /// ```text
    /// func(arg1, , arg3)  // Second argument is None
    /// func(arg1,)         // Second argument is None
    /// ```
    ///
    /// The parsed None argument is returned as `SyscallArg::Ignored`.
    fn parse_none(input: &str) -> IResult<&str, SyscallArg> {
        value(SyscallArg::Ignored, recognize(peek(char(','))))(input)
    }

    /// Parses a signal line.
    ///
    /// Signal lines indicate that a signal was delivered to the process.
    ///
    /// # Format
    ///
    /// ```text
    /// --- SIGTERM {si_signo=SIGTERM, si_code=SI_USER, si_pid=123, si_uid=1000} ---
    /// 123 --- SIGCHLD {si_signo=SIGCHLD, si_code=CLD_EXITED, si_pid=456, si_uid=1000, si_status=0, si_utime=0, si_stime=0} ---
    /// ```
    ///
    /// These lines are skipped during parsing as they don't represent syscalls.
    fn parse_signal_line(input: &str) -> IResult<&str, ()> {
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("---"), take_until("---"), tag("---")),
            )),
        )(input)
    }

    /// Parses an exit status line.
    ///
    /// Exit lines indicate that a process has terminated.
    ///
    /// # Format
    ///
    /// ```text
    /// +++ exited with 0 +++
    /// 123 +++ exited with 1 +++
    /// +++ killed by SIGTERM +++
    /// ```
    ///
    /// These lines are skipped during parsing as they don't represent syscalls.
    fn parse_exit_line(input: &str) -> IResult<&str, ()> {
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("+++"), take_until("+++"), tag("+++")),
            )),
        )(input)
    }

    /// Parses a multithread resumed line.
    ///
    /// In multi-threaded traces, a syscall may be interrupted and resumed later.
    /// The resumed line completes the syscall started in an earlier `<unfinished ...>` line.
    ///
    /// # Format
    ///
    /// ```text
    /// 123 <... openat resumed>) = 3
    /// 456 <... read resumed> "\x00\x01\x02", 1024) = 3
    /// ```
    ///
    /// The parsed PID and the remaining content (after `resumed>`) are returned
    /// as a tuple `(u32, String)`.
    fn parse_multithread_resumed(input: &str) -> IResult<&str, (u32, String)> {
        map(
            tuple((
                Self::parse_pid,
                preceded(
                    space1,
                    tuple((
                        delimited(tag("<..."), take_until("resumed>"), tag("resumed>")),
                        rest,
                    )),
                ),
            )),
            |(pid, (_, remaining))| (pid, remaining.trim().to_string()),
        )(input)
    }

    /// Parses a multithread blocked line.
    ///
    /// In multi-threaded traces, a syscall may be interrupted before completion.
    /// The blocked line contains the syscall name and partial arguments.
    ///
    /// # Format
    ///
    /// ```text
    /// 123 openat(AT_FDCWD</home/user>, "file.txt", O_RDONLY <unfinished ...>
    /// 456 read(3</path/to/file>, <unfinished ...>
    /// ```
    ///
    /// The parsed PID and the content before `<unfinished ...>` are returned
    /// as a tuple `(u32, String)`. This information is stored for later
    /// reconstruction when the corresponding `<... resumed>` line is encountered.
    fn parse_multithread_blocked(input: &str) -> IResult<&str, (u32, String)> {
        map(
            tuple((
                Self::parse_pid,
                preceded(
                    space1,
                    terminated(take_until("<unfinished ...>"), tag("<unfinished ...>")),
                ),
            )),
            |(pid, content)| (pid, content.trim().to_string()),
        )(input)
    }

    /// Gets the syscall name.
    ///
    /// # Returns
    ///
    /// A string slice containing the syscall name (e.g., "openat", "read", "write").
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the original unparsed line.
    ///
    /// # Returns
    ///
    /// The complete original strace output line.
    pub fn original_line(&self) -> &str {
        &self.original_line
    }

    /// Gets the syscall arguments.
    ///
    /// # Returns
    ///
    /// A slice of parsed `SyscallArg` values.
    pub fn args(&self) -> &[SyscallArg] {
        &self.args
    }
}

impl<'a> SyscallArray<'a> {
    /// Creates a new SyscallArray.
    ///
    /// # Arguments
    ///
    /// * `elements` - Vector of syscall arguments of any type
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on successful construction.
    ///
    /// # Example
    ///
    /// ```text
    /// let elements = vec![
    ///     SyscallArg::Integer("123"),
    ///     SyscallArg::String("hello"),
    ///     SyscallArg::Flag("O_RDONLY"),
    /// ];
    /// let array = SyscallArray::new(elements)?;
    /// ```
    pub fn new(elements: Vec<SyscallArg<'a>>) -> Result<Self, StraceParseError> {
        Ok(Self(elements))
    }

    /// Gets the elements in the array.
    ///
    /// # Returns
    ///
    /// A slice of the array elements.
    pub fn elements(&self) -> &[SyscallArg<'a>] {
        &self.0
    }
}

impl<'a> SyscallFlagSet<'a> {
    /// Creates a new SyscallFlagSet with type validation.
    ///
    /// Validates that all elements are either `Flag` or `Integer` types,
    /// as these are the only valid types for flag combinations.
    ///
    /// # Arguments
    ///
    /// * `flags` - Vector of syscall arguments representing flags
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` if all elements are valid flag types, or
    /// `Err(StraceParseError::TypeError)` if any element has an invalid type.
    ///
    /// # Example
    ///
    /// ```text
    /// // Valid: all flags or integers
    /// let flags = vec![
    ///     SyscallArg::Flag("O_RDWR"),
    ///     SyscallArg::Flag("O_CREAT"),
    ///     SyscallArg::Integer("0x200"),
    /// ];
    /// let flag_set = SyscallFlagSet::new(flags)?;
    ///
    /// // Invalid: contains a string
    /// let invalid = vec![
    ///     SyscallArg::Flag("O_RDONLY"),
    ///     SyscallArg::String("invalid"),
    /// ];
    /// assert!(SyscallFlagSet::new(invalid).is_err());
    /// ```
    pub fn new(flags: Vec<SyscallArg<'a>>) -> Result<Self, StraceParseError> {
        for flag in &flags {
            match flag {
                SyscallArg::Flag(_) | SyscallArg::Integer(_) => {}
                _ => {
                    return Err(StraceParseError::TypeError(
                        "SyscallFlagSet elements can only be Flag or Integer types".to_string(),
                    ));
                }
            }
        }

        Ok(Self(flags))
    }

    /// Gets the flags in the set.
    ///
    /// # Returns
    ///
    /// A slice of the flag elements.
    pub fn flags(&self) -> &[SyscallArg] {
        &self.0
    }
}

impl<'a> SyscallStruct<'a> {
    /// Creates a new SyscallStruct.
    ///
    /// # Arguments
    ///
    /// * `fields` - HashMap mapping field names to their corresponding argument values
    ///
    /// # Example
    ///
    /// ```text
    /// let mut fields = HashMap::new();
    /// fields.insert("st_mode", SyscallArg::Flag("S_IFREG|0644"));
    /// fields.insert("st_size", SyscallArg::Integer("1024"));
    /// let struct_arg = SyscallStruct::new(fields);
    /// ```
    pub fn new(fields: HashMap<&'a str, SyscallArg<'a>>) -> Self {
        Self(fields)
    }

    /// Gets all fields in the struct.
    ///
    /// # Returns
    ///
    /// A reference to the HashMap containing all field name-value pairs.
    pub fn fields(&self) -> &HashMap<&'a str, SyscallArg<'a>> {
        &self.0
    }

    /// Gets a specific field value by name.
    ///
    /// # Arguments
    ///
    /// * `key` - The field name to look up
    ///
    /// # Returns
    ///
    /// `Some(&SyscallArg)` if the field exists, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```text
    /// if let Some(mode) = struct_arg.get_field("st_mode") {
    ///     println!("Mode: {:?}", mode);
    /// }
    /// ```
    pub fn get_field(&self, key: &str) -> Option<&SyscallArg> {
        self.0.get(key)
    }
}
