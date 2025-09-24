// SPDX-License-Identifier: MPL-2.0

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
    static BLOCKED_SYSCALL: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

/// Custom error type for strace parsing
#[derive(Debug, Clone, PartialEq)]
pub enum StraceParseError {
    /// A syscall line was blocked and skipped
    BlockedLine,
    /// Signal line was encountered and skipped
    SignalLine,
    /// Exit status line was encountered and skipped
    ExitLine,
    /// Nom parsing error with context
    ParseError { message: String, input: String },
    /// Type validation error
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
#[derive(Debug, PartialEq, Clone)]
pub struct Syscall {
    /// Process ID that made the syscall.
    pid: u32,
    /// Name of the system call.
    name: String,
    /// Arguments passed to the syscall.
    args: Vec<SyscallArg>,
    /// Return value of the syscall.
    /// Since the return value is not important, save all the strings
    /// after `=` to this field, such as `-1 ENOENT (No such file or directory)`.
    return_value: String,
    /// The original input line from strace.
    original_line: String,
}

/// Represents different types of syscall arguments.
#[derive(Debug, PartialEq, Clone)]
pub enum SyscallArg {
    /// Numeric values (integers, hex, etc.).
    Integer(String),
    /// Quoted string literals.
    String(String),
    /// Unquoted string flags or identifiers.
    Flag(String),
    /// File descriptor with associated path.
    FdPath(String),
    /// Combination of flags and/or numbers separated by '|' or " or ".
    Flags(SyscallFlagSet),
    /// Key-value pairs enclosed in {}.
    Struct(SyscallStruct),
    /// List of SyscallArgs enclosed in [].
    Array(SyscallArray),
    /// Arguments that can be ignored for matching purposes.
    Ignored,
}

/// Wrapper for arrays of syscall arguments with type consistency constraint.
/// All elements in the array must be of the same type.
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallArray(Vec<SyscallArg>);

/// Wrapper for flag sets with restricted element types.
/// Elements can only be Flag or Integer types.
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallFlagSet(Vec<SyscallArg>);

/// Wrapper for structured data represented as key-value pairs.
#[derive(Debug, PartialEq, Clone)]
pub struct SyscallStruct(HashMap<String, SyscallArg>);

impl Syscall {
    /// Creates a new Syscall instance
    pub fn new(
        pid: u32,
        name: String,
        args: Vec<SyscallArg>,
        return_value: String,
        original_line: String,
    ) -> Self {
        Self {
            pid,
            name,
            args,
            return_value,
            original_line,
        }
    }

    /// Main interface function to parse strace output into Syscall structure
    ///
    /// # Arguments
    /// * `input` - A single line of strace output
    ///
    /// # Returns
    /// * `Ok(Syscall)` - Successfully parsed syscall
    /// * `Err(StraceParseError)` - Parsing failed with error details
    ///
    /// # Description
    /// Parse strace into a Syscall structure through the nom library.
    /// The syntax tree that complies is as follows:
    /// ```text
    /// Strace Parser Grammar Tree
    /// ==========================
    ///
    /// Root
    /// └── StraceOutput
    ///     ├── SyscallLine
    ///     │   ├── [PID]                                    → u32 (optional)
    ///     │   ├── SyscallName                              → String
    ///     │   ├── Arguments                                → Vec<SyscallArg>
    ///     │   └── ReturnValue                              → String
    ///     │
    ///     ├── MultithreadBlockedLine
    ///     │   ├── PID                                      → u32 (required)
    ///     │   ├── SyscallContent                           → String (incomplete syscall)
    ///     │   └── "<unfinished ...>"                       → marker
    ///     │
    ///     ├── MultithreadResumedLine
    ///     │   ├── PID                                      → u32 (required)
    ///     │   ├── "<..." SyscallName " resumed>"           → marker + syscall name
    ///     │   └── RestContent                              → String (return value part)
    ///     │
    ///     ├── SignalLine (skipped)
    ///     │   └── "---" ... "---"
    ///     │
    ///     └── ExitLine (skipped)
    ///         └── "+++" ... "+++"
    ///
    /// SyscallLine Grammar:
    /// ───────────────────
    /// SyscallLine ::= [WS] [PID WS] SyscallName [WS] Arguments [WS] ReturnValue [WS]
    ///
    /// MultithreadBlockedLine Grammar:
    /// ──────────────────────────────
    /// MultithreadBlockedLine ::= PID WS SyscallContent "<unfinished ...>"
    /// SyscallContent ::= SyscallName [WS] Arguments [WS] (incomplete)
    ///
    /// MultithreadResumedLine Grammar:
    /// ──────────────────────────────
    /// MultithreadResumedLine ::= PID WS "<..." SyscallName " resumed>" RestContent
    /// RestContent ::= [WS] ReturnValue [WS]
    ///
    /// Multithread Processing Flow:
    /// ───────────────────────────
    /// 1. Blocked Line: "1234 openat(AT_FDCWD, \"/path\", O_RDONLY <unfinished ...>"
    ///    └── Store: PID=1234, Content="openat(AT_FDCWD, \"/path\", O_RDONLY"
    ///
    /// 2. Resumed Line: "1234 <... openat resumed>) = 3"
    ///    └── Reconstruct: "1234 openat(AT_FDCWD, \"/path\", O_RDONLY) = 3"
    ///    └── Parse as normal SyscallLine
    ///
    /// PID ::= [0-9]+                                       → u32
    ///
    /// SyscallName ::= [a-zA-Z0-9_]+                        → String
    ///
    /// Arguments ::= '(' [WS] [Argument {[WS] ',' [WS] Argument}*] [WS] ')'    → Vec<SyscallArg>
    ///
    /// Argument ::= [ParamName [WS] '=' [WS]] ArgumentValue [WS] [Comment] [WS] ['=>' [WS] Argument]
    ///
    /// ArgumentValue ::=
    ///     ├── None                                         → SyscallArg::Ignored
    ///     ├── FdPath                                       → SyscallArg::FdPath(String)
    ///     ├── Struct                                       → SyscallArg::Struct(SyscallStruct)
    ///     ├── Array                                        → SyscallArg::Array(SyscallArray)
    ///     ├── QuotedString                                 → SyscallArg::String(String)
    ///     ├── Mask                                         → SyscallArg::Ignored
    ///     ├── Hex                                          → SyscallArg::Integer(String)
    ///     ├── Number                                       → SyscallArg::Integer(String)
    ///     ├── Flags                                        → SyscallArg::Flags(SyscallFlagSet)
    ///     └── UnquotedFlag                                 → SyscallArg::Flag(String)
    ///
    /// ReturnValue ::= '=' [WS] RestOfLine                  → String
    ///
    /// Whitespace Handling:
    /// ───────────────────
    /// WS ::= [ \t]*                                        → ignored
    ///
    /// - Leading and trailing whitespace in the entire input is ignored
    /// - Whitespace around syscall names is ignored
    /// - Whitespace around '=' in parameter assignments is ignored
    /// - Whitespace around commas in argument lists is ignored
    /// - Whitespace around '=' in return values is ignored
    /// - Whitespace around operators ('|', ' or ', '<<', '*') in flags is preserved as part of the token
    /// - Whitespace inside quoted strings is preserved
    /// - Whitespace in comments is preserved but comments are discarded
    ///
    /// Detailed Argument Types:
    /// ─────────────────────────
    ///
    /// 1. None
    ///    ├── Pattern: peek(',' | ')')
    ///    ├── Description: Empty argument between commas
    ///    ├── Whitespace: Leading/trailing whitespace ignored
    ///    └── Example: "func(arg1, , arg3)" → middle arg is None
    ///
    /// 2. FdPath
    ///    ├── Pattern: (Number | 'AT_FDCWD') '<' ... '>'
    ///    ├── Components:
    ///    │   ├── FileDescriptor: Number | "AT_FDCWD"
    ///    │   └── Path: String inside < > (no whitespace trimming)
    ///    ├── Whitespace: No whitespace allowed between fd and '<'
    ///    └── Example: "AT_FDCWD</home/user>" → FdPath("/home/user")
    ///
    /// 3. Hex
    ///    ├── Pattern: '0x' [0-9a-fA-F]+
    ///    ├── Description: Hexadecimal number
    ///    ├── Whitespace: No whitespace allowed within the number
    ///    └── Example: "0x7fff12345678" → Integer("0x7fff12345678")
    ///
    /// 4. Number
    ///    ├── Pattern: ['-'] [0-9]+ [('<<' | '*') ['-'] [0-9]+]
    ///    ├── Components:
    ///    │   ├── Sign: optional '-'
    ///    │   ├── Digits: [0-9]+
    ///    │   └── Operation: optional ('<<' | '*') + optional '-' + [0-9]+
    ///    ├── Whitespace: No whitespace allowed within the number expression
    ///    └── Examples:
    ///        ├── "123" → Integer("123")
    ///        ├── "-456" → Integer("-456")
    ///        ├── "8192*1024" → Integer("8192*1024")
    ///        └── "64<<2" → Integer("64<<2")
    ///
    /// 5. QuotedString
    ///    ├── Pattern: ['@'] '"' ... '"' ['...']
    ///    ├── Components:
    ///    │   ├── Prefix: optional '@'
    ///    │   ├── Content: String with escaped quotes support
    ///    │   └── Suffix: optional '...'
    ///    ├── Whitespace:
    ///    │   ├── No whitespace allowed between '@' and '"'
    ///    │   ├── All whitespace inside quotes is preserved
    ///    │   └── No whitespace allowed between '"' and '...'
    ///    └── Examples:
    ///        ├── '"hello"' → String("hello")
    ///        ├── '@"world"...' → String("world")
    ///        └── '"quote: \"test\""' → String("quote: \"test\"")
    ///
    /// 6. UnquotedFlag
    ///    ├── Pattern: [a-zA-Z0-9_?]+ ['(' ... ')'] ['<<' [0-9]+]
    ///    ├── Components:
    ///    │   ├── Base: [a-zA-Z0-9_?]+
    ///    │   ├── Parameters: optional '(' ... ')' (no whitespace trimming inside)
    ///    │   └── Shift: optional '<<' [0-9]+
    ///    ├── Whitespace: No whitespace allowed between components
    ///    └── Examples:
    ///        ├── "PROT_READ" → Flag("PROT_READ")
    ///        ├── "makedev(0x1, 0x3)" → Flag("makedev(0x1, 0x3)")
    ///        └── "FUTEX_OP_OR<<28" → Flag("FUTEX_OP_OR<<28")
    ///
    /// 7. Flags
    ///    ├── Pattern: (Hex | Number | UnquotedFlag) {(' or ' | '|') (Hex | Number | UnquotedFlag)}*
    ///    ├── Separators:
    ///    │   ├── ' or ' (spaces are part of the separator)
    ///    │   └── '|' (no spaces required)
    ///    ├── Elements: Hex | Number | UnquotedFlag
    ///    ├── Whitespace: No additional whitespace trimming around separators
    ///    └── Examples:
    ///        ├── "PROT_READ|PROT_WRITE" → Flags([Flag("PROT_READ"), Flag("PROT_WRITE")])
    ///        ├── "O_RDONLY or O_CLOEXEC" → Flags([Flag("O_RDONLY"), Flag("O_CLOEXEC")])
    ///        └── "0x1|FLAG<<2|123" → Flags([Integer("0x1"), Flag("FLAG<<2"), Integer("123")])
    ///
    /// 8. Struct
    ///    ├── Pattern: '{' [WS] [StructField {[WS] ',' [WS] StructField}*] [WS] '}'
    ///    ├── StructField: ('...' | Key [WS] '=' [WS] Argument)
    ///    ├── Key: [a-zA-Z0-9_]+
    ///    ├── Whitespace:
    ///    │   ├── Leading/trailing whitespace inside braces is ignored
    ///    │   ├── Whitespace around commas is ignored
    ///    │   └── Whitespace around '=' in field assignments is ignored
    ///    └── Examples:
    ///        ├── "{ st_mode = S_IFDIR|0775 , st_size = 24576 , ... }"
    ///        │   → Struct({
    ///        │       "st_mode": Flags([Flag("S_IFDIR"), Integer("0775")]),
    ///        │       "st_size": Integer("24576")
    ///        │     })
    ///        └── "{...}" → Ignored (incomplete struct)
    ///
    /// 9. Array
    ///    ├── Pattern: '[' [WS] [Argument {[WS] ',' [WS] Argument}*] [WS] ']'
    ///    ├── Elements: Vec<SyscallArg> (type-consistent)
    ///    ├── Whitespace:
    ///    │   ├── Leading/trailing whitespace inside brackets is ignored
    ///    │   └── Whitespace around commas is ignored
    ///    └── Examples:
    ///        ├── '[ "sh" , "-c" , "command" ]'
    ///        │   → Array([String("sh"), String("-c"), String("command")])
    ///        ├── '[ 1 , 2 , 3 ]'
    ///        │   → Array([Integer("1"), Integer("2"), Integer("3")])
    ///        └── '[{WIFEXITED(s) && WEXITSTATUS(s) == 0}]'
    ///        │   → Array([Ignored]) (complex expression as ignored)
    ///
    /// 10. Mask
    ///     ├── Pattern: '~[' ... ']'
    ///     ├── Description: Signal mask or similar constructs
    ///     ├── Whitespace: Content inside brackets is not trimmed
    ///     └── Example: "~[RTMIN RT_1]" → Ignored
    ///
    /// Additional Components:
    /// ─────────────────────
    ///
    /// Comment
    /// ├── Pattern: [WS] '/*' ... '*/' [WS]
    /// ├── Purpose: Explanatory text to be discarded
    /// ├── Whitespace: Leading/trailing whitespace around comment is ignored
    /// └── Example: " /* ARCH_??? */ " → discarded
    ///
    /// ParamName
    /// ├── Pattern: [a-zA-Z0-9_]+
    /// ├── Purpose: Named parameter prefix
    /// ├── Usage: ParamName [WS] '=' [WS] Argument
    /// ├── Whitespace: Whitespace around '=' is ignored
    /// └── Example: "flags = CLONE_VM" → param name "flags" discarded
    ///
    /// Parse Flow Examples:
    /// ──────────────────
    ///
    /// Single-threaded Example:
    /// Input: ' openat( AT_FDCWD</home> , "file.txt" , O_RDONLY ) = 3 '
    ///
    /// Parse Tree:
    /// └── SyscallLine
    ///     ├── PID: None (optional, not present)
    ///     ├── Name: "openat"                       ← leading/trailing WS ignored
    ///     ├── Arguments: [                         ← WS around parens ignored
    ///     │   ├── FdPath("/home")                  ← AT_FDCWD</home>
    ///     │   ├── String("file.txt")               ← "file.txt" (WS around comma ignored)
    ///     │   └── Flags([Flag("O_RDONLY")])        ← O_RDONLY (WS around comma ignored)
    ///     │   ]
    ///     └── ReturnValue: "3"                     ← WS after '=' ignored, trailing WS ignored
    ///
    /// Multi-threaded Example:
    /// Input1: '1234 openat(AT_FDCWD</home>, "file.txt", O_RDONLY <unfinished ...>'
    /// Input2: '1234 <... openat resumed>) = 3'
    ///
    /// Parse Flow:
    /// 1. Input1 → MultithreadBlockedLine
    ///    ├── PID: 1234
    ///    ├── SyscallContent: "openat(AT_FDCWD</home>, \"file.txt\", O_RDONLY"
    ///    └── Store in BLOCKED_SYSCALL[1234]
    ///
    /// 2. Input2 → MultithreadResumedLine
    ///    ├── PID: 1234
    ///    ├── SyscallName: "openat" (extracted from marker)
    ///    ├── RestContent: ") = 3"
    ///    └── Reconstruct: "1234 openat(AT_FDCWD</home>, \"file.txt\", O_RDONLY) = 3"
    ///
    /// 3. Parse reconstructed line as SyscallLine:
    ///    ├── PID: 1234
    ///    ├── Name: "openat"
    ///    ├── Arguments: [FdPath("/home"), String("file.txt"), Flags([Flag("O_RDONLY")])]
    ///    └── ReturnValue: "3"
    ///
    /// Type Constraints:
    /// ───────────────
    /// ├── SyscallArray: All elements must be same type
    /// ├── SyscallFlagSet: Elements can only be Flag or Integer
    /// └── SyscallStruct: Keys are String, values are any SyscallArg
    /// ```
    pub fn parse(input: &str) -> Result<Self, StraceParseError> {
        let mut trimmed = input.trim().to_string();

        // Skip signal lines
        if let Ok(_) = Self::parse_signal_line(&trimmed) {
            return Err(StraceParseError::SignalLine);
        }

        // Skip exit status lines
        if let Ok(_) = Self::parse_exit_line(&trimmed) {
            return Err(StraceParseError::ExitLine);
        }

        // Save blocked syscalls for later reconstruction
        if let Ok((_, (pid, str))) = Self::parse_multithread_blocked(&input) {
            BLOCKED_SYSCALL.with(|blocked| {
                blocked.borrow_mut().insert(pid, str);
            });
            return Err(StraceParseError::BlockedLine);
        }

        // Reconstruct and parse resumed syscalls
        if let Ok((_, (pid, str))) = Self::parse_multithread_resumed(&input) {
            let blocked_call =
                BLOCKED_SYSCALL.with(|blocked| blocked.borrow().get(&pid).cloned().unwrap());
            trimmed = format!("{} {}{}", pid, blocked_call, str);
        }

        match Self::parse_syscall_line(trimmed.as_str()) {
            Ok((_, syscall)) => Ok(syscall),
            Err(e) => Err(StraceParseError::ParseError {
                message: e.to_string(),
                input: trimmed.to_string(),
            }),
        }
    }

    fn parse_syscall_line(input: &str) -> IResult<&str, Syscall> {
        map(
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                Self::parse_syscall_content,
            )),
            |(pid, (name, args, return_value))| {
                Syscall::new(
                    pid.unwrap_or(0),
                    name,
                    args,
                    return_value,
                    input.to_string(),
                )
            },
        )(input)
    }

    fn parse_syscall_content(input: &str) -> IResult<&str, (String, Vec<SyscallArg>, String)> {
        tuple((Self::parse_name, Self::parse_args, Self::parse_return_value))(input)
    }

    /// Parse the syscall name.
    /// The name consists of alphanumeric characters and underscores.
    /// Leading and trailing whitespace is ignored.
    fn parse_name(input: &str) -> IResult<&str, String> {
        map(
            delimited(
                space0,
                take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
                space0,
            ),
            |s: &str| s.to_string(),
        )(input)
    }

    /// Parse the arguments of the syscall.
    /// The arguments are enclosed in parentheses and separated by commas.
    /// Leading and trailing whitespace around each argument is ignored.
    fn parse_args(input: &str) -> IResult<&str, Vec<SyscallArg>> {
        delimited(
            char('('),
            separated_list0(char(','), delimited(space0, Self::parse_arg, space0)),
            char(')'),
        )(input)
    }

    /// Parse the argument of the syscall.
    /// An argument can be one of the following types:
    /// - None: represented by an empty argument (i.e., two consecutive commas or a comma followed by a closing parenthesis)
    /// - FdPath: represented by a file descriptor followed by a path in angle brackets (e.g., `3</path/to/file>`)
    /// - Struct: represented by a key-value pair enclosed in curly braces (e.g., `{key=value, ...}`)
    /// - Array: represented by a list of arguments enclosed in square brackets (e.g., `[arg1, arg2, ...]`)
    /// - QuotedString: represented by a string enclosed in double quotes, optionally prefixed by `@` and suffixed by `...` (e.g., `@"string"...`)
    /// - Mask: represented by a tilde followed by a list of flags enclosed in square brackets (e.g., `~[FLAG1 FLAG2]`)
    /// - Hex: represented by a hexadecimal number prefixed by `0x` (e.g., `0x1A2B3C`)
    /// - Number: represented by a decimal number, optionally prefixed by a minus sign and optionally followed by a shift or multiplication operation (e.g., `-123`, `456<<2`, `789*10`)
    /// - Flags: represented by a combination of Hex, Number, or UnquotedFlag separated by ` or ` or `|` (e.g., `FLAG1|FLAG2 or 0x1A or 123<<2`)
    /// - UnquotedFlag: represented by an alphanumeric string, optionally followed by a parenthetical expression and/or a shift operation (e.g., `FLAG`, `FLAG(param)`, `FLAG<<2`, `FLAG(param)<<2`)
    /// Leading and trailing whitespace around the argument is ignored.
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

    /// Parse the fd path argument.
    /// The fd path consists of a file descriptor (either `AT_FDCWD` or an integer)
    /// followed by a path enclosed in angle brackets.
    /// The parsed path is returned in the `SyscallArg::FdPath` variant.
    fn parse_fd_path(input: &str) -> IResult<&str, SyscallArg> {
        map(
            tuple((
                alt((tag("AT_FDCWD"), take_while1(|c: char| c.is_ascii_digit()))),
                delimited(
                    char::<&str, nom::error::Error<&str>>('<'),
                    take_until(">"),
                    char::<&str, nom::error::Error<&str>>('>'),
                ),
            )),
            |(_, path)| SyscallArg::FdPath(path.to_string()),
        )(input)
    }

    /// Parse a quoted string argument.
    /// The string is enclosed in double quotes, and may be optionally prefixed by `@`
    /// and suffixed by `...`.
    /// The parsed string is returned in the `SyscallArg::String` variant.
    fn parse_quoted_string(input: &str) -> IResult<&str, SyscallArg> {
        map(
            tuple((
                opt(char('@')), // Optional @ prefix
                delimited(char('"'), Self::take_until_unescaped_quote, char('"')),
                opt(tag("...")), // Optional ... suffix
            )),
            |(_, content, _)| SyscallArg::String(content.to_string()),
        )(input)
    }

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

    /// Parse a hexadecimal number argument.
    /// The number is prefixed by `0x` and consists of hexadecimal digits.
    /// The parsed number is returned as a string in the `SyscallArg::Integer` variant.
    fn parse_hex(input: &str) -> IResult<&str, SyscallArg> {
        map(
            recognize(preceded(
                tag("0x"),
                take_while1(|c: char| c.is_ascii_hexdigit()),
            )),
            |s: &str| SyscallArg::Integer(s.to_string()),
        )(input)
    }

    /// Parse a decimal number argument.
    /// The number consists of decimal digits, and may be optionally prefixed by a minus sign
    /// and optionally followed by a shift or multiplication operation.
    /// The parsed number is returned as a string in the `SyscallArg::Integer` variant.
    fn parse_number(input: &str) -> IResult<&str, SyscallArg> {
        map(
            recognize(tuple((
                opt(char('-')),
                digit1,
                opt(tuple((alt((tag("<<"), tag("*"))), opt(char('-')), digit1))),
            ))),
            |s: &str| SyscallArg::Integer(s.to_string()),
        )(input)
    }

    /// Parse a set of flags.
    /// The flags are a combination of Hex, Number, or UnquotedFlag separated by ` or ` or `|`.
    /// The parsed flags are returned as a `SyscallArg::Flags` variant containing a `SyscallFlagSet`.
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

    /// Parse an unquoted flag argument.
    /// The flag consists of an alphanumeric string, optionally followed by a parenthetical expression
    /// and/or a shift operation.
    /// The parsed flag is returned as a string in the `SyscallArg::Flag` variant.
    fn parse_unquoted_flag(input: &str) -> IResult<&str, SyscallArg> {
        map(
            tuple((
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
            )),
            |(base, paren_content, shift_content)| {
                let paren_part =
                    paren_content.map_or(String::new(), |content| format!("({})", content));
                let shift_part =
                    shift_content.map_or(String::new(), |(op, val)| format!("{}{}", op, val));
                SyscallArg::Flag(format!("{}{}{}", base, paren_part, shift_part))
            },
        )(input)
    }

    /// Parse a mask argument.
    /// The mask consists of a tilde followed by a list of flags enclosed in square brackets.
    /// The parsed mask is returned as `SyscallArg::Ignored`.
    fn parse_mask(input: &str) -> IResult<&str, SyscallArg> {
        value(
            SyscallArg::Ignored,
            delimited(tag("~["), take_until("]"), char(']')),
        )(input)
    }

    /// Parse a struct argument.
    /// The struct consists of key-value pairs enclosed in curly braces.
    /// Each key-value pair is separated by a comma, and the value is an argument which should be parsed recursively.
    /// If the struct contains `...`, it is treated as ignored.
    /// If the struct cannot be parsed, it is treated as ignored.
    /// The parsed struct is returned as a `SyscallArg::Struct` variant containing a `SyscallStruct`.
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
                                    |(k, v)| Some((k.to_string(), v)),
                                ),
                            )),
                        ),
                    ),
                    char('}'),
                ),
                |pairs| {
                    let fields: HashMap<String, SyscallArg> =
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

    /// Parse an array argument.
    /// The array consists of arguments enclosed in square brackets, separated by commas.
    /// Each element should be parsed recursively.
    /// The space around the commas is ignored.
    /// If the array cannot be parsed, it is treated as ignored, i.e. `SyscallArg::Ignored`.
    /// The parsed array is returned as a `SyscallArg::Array` variant containing a `SyscallArray`.
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

    /// Parse the return value of the syscall.
    /// The return value is prefixed by `=` and consists of the rest of the line.
    /// The parsed return value is returned as a string.
    fn parse_return_value(input: &str) -> IResult<&str, String> {
        preceded(
            tuple((space0, char('='), space0)),
            map(rest, |s: &str| s.trim().to_string()),
        )(input)
    }

    /// Parse a PID, which is a sequence of digits, and convert it to u32.
    fn parse_pid(input: &str) -> IResult<&str, u32> {
        map(digit1, |s: &str| s.parse::<u32>().unwrap())(input)
    }

    /// Parse a None argument, which is represented by an empty argument.
    /// This occurs when there are two consecutive commas.
    /// The parsed None argument is returned as `SyscallArg::Ignored`.
    fn parse_none(input: &str) -> IResult<&str, SyscallArg> {
        value(SyscallArg::Ignored, recognize(peek(char(','))))(input)
    }

    /// Parse a signal line.
    /// Signal lines have the pattern: "--- SIGNAME {...} ---"
    /// These lines are skipped during parsing.
    fn parse_signal_line(input: &str) -> IResult<&str, ()> {
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("---"), take_until("---"), tag("---")),
            )),
        )(input)
    }

    /// Parse an exit status line.
    /// Exit lines have the pattern: "+++ exited with N +++" or similar
    /// These lines are skipped during parsing.
    fn parse_exit_line(input: &str) -> IResult<&str, ()> {
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("+++"), take_until("+++"), tag("+++")),
            )),
        )(input)
    }

    /// Parse a multithread resumed line.
    /// The line starts with a PID followed by a space and then <... xxx resumed> and the rest of the line.
    /// The parsed PID and the rest of the line are returned as a tuple (u32, String).
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

    /// Parse a multithread blocked line.
    /// The line starts with a PID followed by a space and then the rest of the line
    /// until <unfinished ...> and then <unfinished ...>.
    /// The parsed PID and the rest of the line (excluding the <unfinished ...> part)
    /// are returned as a tuple (u32, String).
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

    /// Gets the syscall name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the original line
    pub fn original_line(&self) -> &str {
        &self.original_line
    }

    /// Gets the syscall arguments
    pub fn args(&self) -> &[SyscallArg] {
        &self.args
    }
}

impl SyscallArray {
    /// Creates a new SyscallArray with type consistency validation.
    /// Check all elements in the array must be of the same type.
    pub fn new(elements: Vec<SyscallArg>) -> Result<Self, StraceParseError> {
        if elements.is_empty() {
            return Ok(Self(elements));
        }

        // Check type consistency - all elements should be the same variant
        let first_type = std::mem::discriminant(&elements[0]);
        for element in &elements[1..] {
            if std::mem::discriminant(element) != first_type {
                return Err(StraceParseError::TypeError(format!(
                    "All elements in SyscallArray must be of the same type: {:?}",
                    elements
                )));
            }
        }

        Ok(Self(elements))
    }

    /// Gets the elements in the array
    pub fn elements(&self) -> &[SyscallArg] {
        &self.0
    }
}

impl SyscallFlagSet {
    /// Creates a new SyscallFlagSet with type validation.
    /// Check that all elements are either Flag or Integer.
    pub fn new(flags: Vec<SyscallArg>) -> Result<Self, StraceParseError> {
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

    /// Gets the flags in the set
    pub fn flags(&self) -> &[SyscallArg] {
        &self.0
    }
}

impl SyscallStruct {
    /// Creates a new SyscallStruct
    pub fn new(fields: HashMap<String, SyscallArg>) -> Self {
        Self(fields)
    }

    /// Gets the fields in the struct
    pub fn fields(&self) -> &HashMap<String, SyscallArg> {
        &self.0
    }

    /// Gets a specific field value
    pub fn get_field(&self, key: &str) -> Option<&SyscallArg> {
        self.0.get(key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn test_parse_mmap_syscall() {
        let line = "mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0x7efe0a77a000".to_string();
        let result = Syscall::parse(line.as_str()).unwrap();
        let expected = Syscall::new(
            0,
            "mmap".to_string(),
            vec![
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
                SyscallArg::Integer("8192".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("PROT_READ".to_string()),
                        SyscallArg::Flag("PROT_WRITE".to_string()),
                    ])
                    .unwrap(),
                ),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("MAP_PRIVATE".to_string()),
                        SyscallArg::Flag("MAP_ANONYMOUS".to_string()),
                    ])
                    .unwrap(),
                ),
                SyscallArg::Integer("-1".to_string()),
                SyscallArg::Integer("0".to_string()),
            ],
            "0x7efe0a77a000".to_string(),
            line,
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_openat_syscall_with_fdpath() {
        let line = "openat(AT_FDCWD</home/sutao/sctrace>, \"/home/sutao/sctrace/target/debug/deps/tls/x86_64/x86_64/libc.so.6\", O_RDONLY|O_CLOEXEC) = -1 ENOENT (No such file or directory)".to_string();
        let result = Syscall::parse(line.as_str()).unwrap();
        let expected = Syscall::new(
            0,
            "openat".to_string(),
            vec![
                SyscallArg::FdPath("/home/sutao/sctrace".to_string()),
                SyscallArg::String(
                    "/home/sutao/sctrace/target/debug/deps/tls/x86_64/x86_64/libc.so.6".to_string(),
                ),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("O_RDONLY".to_string()),
                        SyscallArg::Flag("O_CLOEXEC".to_string()),
                    ])
                    .unwrap(),
                ),
            ],
            "-1 ENOENT (No such file or directory)".to_string(),
            line,
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_arch_prctl_syscall_with_comments() {
        let line =
            "arch_prctl(0x3001 /* ARCH_??? */, 0x7ffdc5d7ade0) = -1 EINVAL (Invalid argument)"
                .to_string();
        let result = Syscall::parse(line.as_str()).unwrap();
        let expected = Syscall::new(
            0,
            "arch_prctl".to_string(),
            vec![
                SyscallArg::Integer("0x3001".to_string()),
                SyscallArg::Integer("0x7ffdc5d7ade0".to_string()),
            ],
            "-1 EINVAL (Invalid argument)".to_string(),
            line,
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_stat_syscall_with_struct() {
        let line = "stat(\"/home/sutao/sctrace/target/debug/deps\", {st_mode=S_IFDIR|0775, st_size=24576, ...}) = 0".to_string();
        let result = Syscall::parse(line.as_str()).unwrap();
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "st_mode".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("S_IFDIR".to_string()),
                    SyscallArg::Integer("0775".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "st_size".to_string(),
            SyscallArg::Integer("24576".to_string()),
        );
        let expected = Syscall::new(
            0,
            "stat".to_string(),
            vec![
                SyscallArg::String("/home/sutao/sctrace/target/debug/deps".to_string()),
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_execve_syscall_with_array() {
        let line = "execve(\"/usr/bin/sh\", [\"sh\", \"-c\", \"/home/sutao/test/hello_world >/d\"...], 0x7fff398783d8 /* 59 vars */) = 0".to_string();
        let result = Syscall::parse(line.as_str()).unwrap();
        let expected = Syscall::new(
            0,
            "execve".to_string(),
            vec![
                SyscallArg::String("/usr/bin/sh".to_string()),
                SyscallArg::Array(
                    SyscallArray::new(vec![
                        SyscallArg::String("sh".to_string()),
                        SyscallArg::String("-c".to_string()),
                        SyscallArg::String("/home/sutao/test/hello_world >/d".to_string()),
                    ])
                    .unwrap(),
                ),
                SyscallArg::Integer("0x7fff398783d8".to_string()),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_signal_line_returns_error() {
        let result = Syscall::parse(
            "--- SIGCHLD {si_signo=SIGCHLD, si_code=CLD_EXITED, si_pid=1811731, si_uid=1012, si_status=0, si_utime=0, si_stime=0} ---",
        );
        assert!(result.is_err());
        assert_eq!(result, Err(StraceParseError::SignalLine));
    }

    #[test]
    fn test_parse_exit_status_line_returns_error() {
        let result = Syscall::parse("+++ exited with 0 +++");
        assert!(result.is_err());
        assert_eq!(result, Err(StraceParseError::ExitLine));
    }

    #[test]
    fn test_parse_fstat_syscall_with_special_flag() {
        let line =
            "fstat(3,{st_mode=S_IFCHR|0666, st_rdev=makedev(0x1, 0x3), ...}) = 0".to_string();
        let result = Syscall::parse(line.as_str());
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "st_mode".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("S_IFCHR".to_string()),
                    SyscallArg::Integer("0666".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "st_rdev".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![SyscallArg::Flag("makedev(0x1, 0x3)".to_string())])
                    .unwrap(),
            ),
        );
        let expected = Syscall::new(
            0,
            "fstat".to_string(),
            vec![
                SyscallArg::Integer("3".to_string()),
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_read_syscall_with_escaped_quotes() {
        let line =
            "read(3, \".less-history-file:\\n.search\\n\\\\\\\"v6.\"..., 4096) = 150".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "read".to_string(),
            vec![
                SyscallArg::Integer("3".to_string()),
                SyscallArg::String(".less-history-file:\\n.search\\n\\\\\\\"v6.".to_string()),
                SyscallArg::Integer("4096".to_string()),
            ],
            "150".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_clone3_syscall_with_output_redirection() {
        let line = "clone3({flags=CLONE_VM|CLONE_FS|CLONE_FILES|CLONE_SIGHAND|CLONE_THREAD|CLONE_SYSVSEM|CLONE_SETTLS|CLONE_PARENT_SETTID|CLONE_CHILD_CLEARTID, child_tid=0x7b57d9600990, parent_tid=0x7b57d9600990, exit_signal=0, stack=0x7b57d8e00000, stack_size=0x7ffc00, tls=0x7b57d96006c0} => {parent_tid=[141626]}, 88) = 141626".to_string();
        let result = Syscall::parse(line.as_str());
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "flags".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("CLONE_VM".to_string()),
                    SyscallArg::Flag("CLONE_FS".to_string()),
                    SyscallArg::Flag("CLONE_FILES".to_string()),
                    SyscallArg::Flag("CLONE_SIGHAND".to_string()),
                    SyscallArg::Flag("CLONE_THREAD".to_string()),
                    SyscallArg::Flag("CLONE_SYSVSEM".to_string()),
                    SyscallArg::Flag("CLONE_SETTLS".to_string()),
                    SyscallArg::Flag("CLONE_PARENT_SETTID".to_string()),
                    SyscallArg::Flag("CLONE_CHILD_CLEARTID".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "child_tid".to_string(),
            SyscallArg::Integer("0x7b57d9600990".to_string()),
        );
        struct_fields.insert(
            "parent_tid".to_string(),
            SyscallArg::Integer("0x7b57d9600990".to_string()),
        );
        struct_fields.insert(
            "exit_signal".to_string(),
            SyscallArg::Integer("0".to_string()),
        );
        struct_fields.insert(
            "stack".to_string(),
            SyscallArg::Integer("0x7b57d8e00000".to_string()),
        );
        struct_fields.insert(
            "stack_size".to_string(),
            SyscallArg::Integer("0x7ffc00".to_string()),
        );
        struct_fields.insert(
            "tls".to_string(),
            SyscallArg::Integer("0x7b57d96006c0".to_string()),
        );
        let expected = Syscall::new(
            0,
            "clone3".to_string(),
            vec![
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
                SyscallArg::Integer("88".to_string()),
            ],
            "141626".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_futex_syscall_with_shift_operator() {
        let line = "futex(0x7ffd737a9f20, FUTEX_WAKE_OP_PRIVATE, 1, 2147483647, 0x7ffd737a9f24, FUTEX_OP_OR<<28|0<<12|FUTEX_OP_CMP_NE<<24|0)             = 1".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "futex".to_string(),
            vec![
                SyscallArg::Integer("0x7ffd737a9f20".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag(
                        "FUTEX_WAKE_OP_PRIVATE".to_string(),
                    )])
                    .unwrap(),
                ),
                SyscallArg::Integer("1".to_string()),
                SyscallArg::Integer("2147483647".to_string()),
                SyscallArg::Integer("0x7ffd737a9f24".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("FUTEX_OP_OR<<28".to_string()),
                        SyscallArg::Integer("0<<12".to_string()),
                        SyscallArg::Flag("FUTEX_OP_CMP_NE<<24".to_string()),
                        SyscallArg::Integer("0".to_string()),
                    ])
                    .unwrap(),
                ),
            ],
            "1".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_ioctl_syscall_with_or_flags() {
        let line = "ioctl(0, SNDCTL_TMR_STOP or TCSETSW, {c_iflag=ICRNL|IXON|IUTF8, c_oflag=NL0|CR0|TAB0|BS0|VT0|FF0|OPOST|ONLCR, c_cflag=B38400|CS8|CREAD, c_lflag=ISIG|ICANON|ECHO|ECHOE|ECHOK|IEXTEN|ECHOCTL|ECHOKE, ...})             = -1 EIO (Input/output error)".to_string();
        let result = Syscall::parse(line.as_str());
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "c_iflag".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("ICRNL".to_string()),
                    SyscallArg::Flag("IXON".to_string()),
                    SyscallArg::Flag("IUTF8".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "c_oflag".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("NL0".to_string()),
                    SyscallArg::Flag("CR0".to_string()),
                    SyscallArg::Flag("TAB0".to_string()),
                    SyscallArg::Flag("BS0".to_string()),
                    SyscallArg::Flag("VT0".to_string()),
                    SyscallArg::Flag("FF0".to_string()),
                    SyscallArg::Flag("OPOST".to_string()),
                    SyscallArg::Flag("ONLCR".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "c_cflag".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("B38400".to_string()),
                    SyscallArg::Flag("CS8".to_string()),
                    SyscallArg::Flag("CREAD".to_string()),
                ])
                .unwrap(),
            ),
        );
        struct_fields.insert(
            "c_lflag".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![
                    SyscallArg::Flag("ISIG".to_string()),
                    SyscallArg::Flag("ICANON".to_string()),
                    SyscallArg::Flag("ECHO".to_string()),
                    SyscallArg::Flag("ECHOE".to_string()),
                    SyscallArg::Flag("ECHOK".to_string()),
                    SyscallArg::Flag("IEXTEN".to_string()),
                    SyscallArg::Flag("ECHOCTL".to_string()),
                    SyscallArg::Flag("ECHOKE".to_string()),
                ])
                .unwrap(),
            ),
        );
        let expected = Syscall::new(
            0,
            "ioctl".to_string(),
            vec![
                SyscallArg::Integer("0".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("SNDCTL_TMR_STOP".to_string()),
                        SyscallArg::Flag("TCSETSW".to_string()),
                    ])
                    .unwrap(),
                ),
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
            ],
            "-1 EIO (Input/output error)".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_prlimit64_syscall_with_multiplying_numbers() {
        let line =
            "prlimit64(0, RLIMIT_STACK, NULL, {rlim_cur=8192*1024, rlim_max=RLIM64_INFINITY}) = 0"
                .to_string();
        let result = Syscall::parse(line.as_str());
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "rlim_cur".to_string(),
            SyscallArg::Integer("8192*1024".to_string()),
        );
        struct_fields.insert(
            "rlim_max".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![SyscallArg::Flag("RLIM64_INFINITY".to_string())]).unwrap(),
            ),
        );
        let expected = Syscall::new(
            0,
            "prlimit64".to_string(),
            vec![
                SyscallArg::Integer("0".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("RLIMIT_STACK".to_string())])
                        .unwrap(),
                ),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_setsockopt_syscall_with_empty_argument() {
        let line =
            "setsockopt(3, SOL_SOCKET, , [8388608], 4)        = -1 EPERM (Operation not permitted)"
                .to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "setsockopt".to_string(),
            vec![
                SyscallArg::Integer("3".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("SOL_SOCKET".to_string())]).unwrap(),
                ),
                SyscallArg::Ignored,
                SyscallArg::Array(
                    SyscallArray::new(vec![SyscallArg::Integer("8388608".to_string())]).unwrap(),
                ),
                SyscallArg::Integer("4".to_string()),
            ],
            "-1 EPERM (Operation not permitted)".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_rt_sigprocmask_syscall_with_mask_argument() {
        let line = "rt_sigprocmask(SIG_UNBLOCK, [RTMIN RT_1], NULL, 8) = 0".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "rt_sigprocmask".to_string(),
            vec![
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("SIG_UNBLOCK".to_string())]).unwrap(),
                ),
                SyscallArg::Ignored,
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
                SyscallArg::Integer("8".to_string()),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_rt_sigaction_syscall_with_mask_argument() {
        let line = "rt_sigaction(SIGCHLD, {sa_handler=0x6289f0868cd0, sa_mask=~[RTMIN RT_1], sa_flags=SA_RESTORER, sa_restorer=0x7f7745a45330}, NULL, 8) = 0".to_string();
        let result = Syscall::parse(line.as_str());
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "sa_handler".to_string(),
            SyscallArg::Integer("0x6289f0868cd0".to_string()),
        );
        struct_fields.insert("sa_mask".to_string(), SyscallArg::Ignored);
        struct_fields.insert(
            "sa_flags".to_string(),
            SyscallArg::Flags(
                SyscallFlagSet::new(vec![SyscallArg::Flag("SA_RESTORER".to_string())]).unwrap(),
            ),
        );
        struct_fields.insert(
            "sa_restorer".to_string(),
            SyscallArg::Integer("0x7f7745a45330".to_string()),
        );
        let expected = Syscall::new(
            0,
            "rt_sigaction".to_string(),
            vec![
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("SIGCHLD".to_string())]).unwrap(),
                ),
                SyscallArg::Struct(SyscallStruct::new(struct_fields)),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
                SyscallArg::Integer("8".to_string()),
            ],
            "0".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_wait4_syscall_with_expression_argument() {
        let line =
            "wait4(-1,[{WIFEXITED(s) && WEXITSTATUS(s) == 0}], 0, NULL) = 141612".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "wait4".to_string(),
            vec![
                SyscallArg::Integer("-1".to_string()),
                SyscallArg::Array(SyscallArray::new(vec![SyscallArg::Ignored]).unwrap()),
                SyscallArg::Integer("0".to_string()),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
            ],
            "141612".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_clone_syscall_with_named_parameters() {
        let line = "clone(child_stack=NULL, flags=CLONE_CHILD_CLEARTID|CLONE_CHILD_SETTID|SIGCHLD, child_tidptr=0x7f7745c1ca10) = 141614".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(
            0,
            "clone".to_string(),
            vec![
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![SyscallArg::Flag("NULL".to_string())]).unwrap(),
                ),
                SyscallArg::Flags(
                    SyscallFlagSet::new(vec![
                        SyscallArg::Flag("CLONE_CHILD_CLEARTID".to_string()),
                        SyscallArg::Flag("CLONE_CHILD_SETTID".to_string()),
                        SyscallArg::Flag("SIGCHLD".to_string()),
                    ])
                    .unwrap(),
                ),
                SyscallArg::Integer("0x7f7745c1ca10".to_string()),
            ],
            "141614".to_string(),
            line,
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_without_argument() {
        let line = "getuid()                        = 1012".to_string();
        let result = Syscall::parse(line.as_str());
        let expected = Syscall::new(0, "getuid".to_string(), vec![], "1012".to_string(), line);
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_multithread_complex_syscall() {
        BLOCKED_SYSCALL.with(|blocked| {
            blocked.borrow_mut().insert(
                9999,
                "mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1".to_string(),
            );
        });

        let line = "9999 <... mmap resumed>, 0) = 0x7efe0a77a000".to_string();
        let result = Syscall::parse(line.as_str());

        let expected = Syscall::new(9999, "mmap".to_string(),
            vec![SyscallArg::Flags(SyscallFlagSet::new(vec![
                     SyscallArg::Flag("NULL".to_string())
                 ]).unwrap()),
                 SyscallArg::Integer("8192".to_string()),
                 SyscallArg::Flags(SyscallFlagSet::new(vec![
                     SyscallArg::Flag("PROT_READ".to_string()),
                     SyscallArg::Flag("PROT_WRITE".to_string())
                 ]).unwrap()),
                 SyscallArg::Flags(SyscallFlagSet::new(vec![
                     SyscallArg::Flag("MAP_PRIVATE".to_string()),
                     SyscallArg::Flag("MAP_ANONYMOUS".to_string())
                 ]).unwrap()),
                 SyscallArg::Integer("-1".to_string()),
                 SyscallArg::Integer("0".to_string()),
            ],
            "0x7efe0a77a000".to_string(),
            "9999 mmap(NULL, 8192, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0) = 0x7efe0a77a000".to_string());
        assert_eq!(result.unwrap(), expected);
    }
}
