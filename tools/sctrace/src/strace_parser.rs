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
    /// Storage for blocked syscalls by PID in multi-threaded strace output.
    static BLOCKED_SYSCALL: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StraceParseError {
    BlockedLine,
    SignalLine,
    ExitLine,
    EmptyLine,
    ParseError { message: String, input: String },
    TypeError(String),
}

impl fmt::Display for StraceParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StraceParseError::BlockedLine => write!(f, "Error: Blocked syscall line"),
            StraceParseError::SignalLine => write!(f, "Error: Signal line"),
            StraceParseError::ExitLine => write!(f, "Error: Exit status line"),
            StraceParseError::EmptyLine => write!(f, "Error: Empty line"),
            StraceParseError::ParseError { message, input } => {
                write!(f, "{} (input: {})", message, input)
            }
            StraceParseError::TypeError(msg) => write!(f, "Type error: {}", msg),
        }
    }
}

impl Error for StraceParseError {}

/// Syscall representation parsed from strace output.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Syscall<'a> {
    pid: u32,
    name: &'a str,
    args: Vec<SyscallArg<'a>>,
    return_value: &'a str,
    original_line: &'a str,
}

impl<'a> Syscall<'a> {
    /// Fetches and preprocesses a strace line before parsing.
    pub(crate) fn fetch(line: String) -> Result<String, StraceParseError> {
        let trimmed = line.as_str().trim();

        if trimmed.is_empty() {
            return Err(StraceParseError::EmptyLine);
        }

        // Skip signal lines
        if Self::parse_signal_line(trimmed).is_ok() {
            return Err(StraceParseError::SignalLine);
        }

        // Skip exit status lines
        if Self::parse_exit_line(trimmed).is_ok() {
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
    pub(crate) fn parse(input: &'a str) -> Result<Self, StraceParseError> {
        let trimmed = input.trim();

        let syscall = Self::parse_syscall(trimmed)
            .map(|(_, syscall)| syscall)
            .map_err(|e| StraceParseError::ParseError {
                message: e.to_string(),
                input: trimmed.to_string(),
            })?;

        let syscall = Self::handle_special_cases(syscall);
        Ok(syscall)
    }

    pub(crate) fn name(&self) -> &str {
        self.name
    }

    pub(crate) fn original_line(&self) -> &str {
        self.original_line
    }

    pub(crate) fn args(&self) -> &[SyscallArg<'_>] {
        &self.args
    }

    fn new(
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
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum SyscallArg<'a> {
    /// Integer argument represented as a string.
    Integer(&'a str),

    /// Quoted string argument.
    String(&'a str),

    /// Unquoted flag argument.
    Flag(&'a str),

    /// File descriptor with absolute path.
    FdPath(&'a str),

    /// Combination of flags and/or integer values.
    Flags(SyscallFlagSet<'a>),

    /// Structured data represented as key-value pairs.
    Struct(SyscallStruct<'a>),

    /// Array of syscall arguments.
    Array(SyscallArray<'a>),

    /// Argument is ignored.
    Ignored,
}

/// Wrapper for arrays of syscall arguments.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SyscallArray<'a>(Vec<SyscallArg<'a>>);

impl<'a> SyscallArray<'a> {
    pub(crate) fn elements(&self) -> &[SyscallArg<'a>] {
        &self.0
    }

    fn new(elements: Vec<SyscallArg<'a>>) -> Result<Self, StraceParseError> {
        Ok(Self(elements))
    }
}

/// Wrapper for flag sets with restricted element types.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SyscallFlagSet<'a>(Vec<SyscallArg<'a>>);

impl<'a> SyscallFlagSet<'a> {
    pub(crate) fn flags(&self) -> &[SyscallArg<'_>] {
        &self.0
    }

    fn new(flags: Vec<SyscallArg<'a>>) -> Result<Self, StraceParseError> {
        // Validates that all elements are either `Flag` or `Integer` types,
        // as these are the only valid types for flag combinations.
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
}

/// Wrapper for structured data represented as key-value pairs.
#[derive(Debug, PartialEq, Clone)]
pub(crate) struct SyscallStruct<'a>(HashMap<&'a str, SyscallArg<'a>>);

impl<'a> SyscallStruct<'a> {
    pub(crate) fn fields(&self) -> &HashMap<&'a str, SyscallArg<'a>> {
        &self.0
    }

    pub(crate) fn get_value(&self, key: &str) -> Option<&SyscallArg<'_>> {
        self.0.get(key)
    }

    fn new(fields: HashMap<&'a str, SyscallArg<'a>>) -> Self {
        Self(fields)
    }
}

impl Syscall<'_> {
    fn parse_syscall(input: &str) -> IResult<&str, Syscall<'_>> {
        let original_input = input;
        let (input, _) = space0(input)?;
        let (input, pid) = opt(terminated(Self::parse_pid, space1))(input)?;
        let (input, _) = space0(input)?;
        let (input, name) = Self::parse_name(input)?;
        let (input, _) = space0(input)?;
        let (input, args) = Self::parse_args(input)?;
        let (input, _) = space0(input)?;
        let (input, return_value) = Self::parse_return_value(input)?;

        Ok((
            input,
            Syscall::new(pid.unwrap_or(0), name, args, return_value, original_input),
        ))
    }

    fn parse_name(input: &str) -> IResult<&str, &str> {
        delimited(
            space0,
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
            space0,
        )(input)
    }

    fn parse_args(input: &str) -> IResult<&str, Vec<SyscallArg<'_>>> {
        delimited(
            char('('),
            separated_list0(char(','), delimited(space0, Self::parse_arg, space0)),
            char(')'),
        )(input)
    }

    fn parse_arg(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Skip parameter's name
        let (input, _) = opt(terminated(
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
            char('='),
        ))(input)?;

        let (input, arg) = alt((
            Self::parse_none,
            Self::parse_fd_path,
            Self::parse_struct,
            Self::parse_array,
            Self::parse_quoted_string,
            Self::parse_mask,
            Self::parse_hex,
            Self::parse_number,
            Self::parse_flags,
        ))(input)?;

        // Skip comment
        let (input, _) = opt(delimited(
            delimited(space0, tag("/*"), space0),
            take_until("*/"),
            tag("*/"),
        ))(input)?;

        // Skip output parameter with arrow
        let (input, _) = opt(preceded(
            delimited(space0, tag("=>"), space0),
            Self::parse_arg,
        ))(input)?;

        Ok((input, arg))
    }

    /// Parses a file descriptor or `AT_FDCWD` with absolute path argument.
    fn parse_fd_path(input: &str) -> IResult<&str, SyscallArg<'_>> {
        let (input, _fd) =
            alt((tag("AT_FDCWD"), take_while1(|c: char| c.is_ascii_digit())))(input)?;
        let (input, path) = Self::parse_angle_bracket_content(input)?;

        Ok((input, SyscallArg::FdPath(path)))
    }

    /// Parses content within angle brackets, handling nested brackets and arrows.
    fn parse_angle_bracket_content(input: &str) -> IResult<&str, &str> {
        let (input, _) = char::<&str, nom::error::Error<&str>>('<')(input)?;

        let mut depth = 1;
        let mut end_pos = 0;
        let chars = input.char_indices().peekable();

        for (i, ch) in chars {
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
    fn parse_quoted_string(input: &str) -> IResult<&str, SyscallArg<'_>> {
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
    fn take_until_unescaped_quote(input: &str) -> IResult<&str, &str> {
        let chars = input.char_indices();
        let mut last_was_escape = false;

        for (i, ch) in chars {
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

    fn parse_hex(input: &str) -> IResult<&str, SyscallArg<'_>> {
        map(
            recognize(preceded(
                tag("0x"),
                take_while1(|c: char| c.is_ascii_hexdigit()),
            )),
            |s: &str| SyscallArg::Integer(s),
        )(input)
    }

    fn parse_number(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Supports negative numbers and arithmetic operations like left shift
        // and multiplication.
        //
        // First, check if this looks like `number<<FLAG` pattern
        // If so, fail early to let other parsers handle it
        if let Ok((remaining, _)) =
            recognize::<_, _, nom::error::Error<&str>, _>(tuple((digit1, tag("<<"))))(input)
        {
            // Check if what follows is an identifier (not a digit)
            if peek(take_while1::<_, _, nom::error::Error<&str>>(|c: char| {
                c.is_ascii_alphabetic() || c == '_'
            }))(remaining)
            .is_ok()
            {
                // This is `number<<FLAG` format, reject it
                return Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Verify,
                )));
            }
        }

        map(
            recognize(tuple((
                opt(char('-')),
                digit1,
                opt(tuple((alt((tag("<<"), tag("*"))), opt(char('-')), digit1))),
            ))),
            |s: &str| SyscallArg::Integer(s),
        )(input)
    }

    fn parse_flags(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Flags can be separated by `|` or ` or `
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

    fn parse_unquoted_flag(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Flags are symbolic constants that may optionally include:
        // - Parenthetical parameters: `FLAG(param)`
        // - Left shift operations: `FLAG<<2` and `1<<FLAG`
        alt((
            map(
                recognize(tuple((
                    take_while1(|c: char| c.is_ascii_digit()),
                    tag("<<"),
                    take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '?'),
                ))),
                |matched: &str| SyscallArg::Flag(matched),
            ),
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
            ),
        ))(input)
    }

    fn parse_mask(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Parse the format like `~[flags]`
        value(
            SyscallArg::Ignored,
            delimited(tag("~["), take_until("]"), char(']')),
        )(input)
    }

    fn parse_struct(input: &str) -> IResult<&str, SyscallArg<'_>> {
        let (input, _) = char('{')(input)?;
        let (input, pairs) = separated_list0(
            char(','),
            preceded(
                space0,
                alt((
                    value(None, tag("...")),
                    map(
                        separated_pair(
                            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_'),
                            char('='),
                            Self::parse_arg,
                        ),
                        |(k, v)| Some((k, v)),
                    ),
                )),
            ),
        )(input)?;
        let (input, _) = char('}')(input)?;

        let fields: HashMap<&str, SyscallArg> = pairs.into_iter().flatten().collect();
        let result = if fields.is_empty() {
            SyscallArg::Ignored
        } else {
            SyscallArg::Struct(SyscallStruct::new(fields))
        };

        Ok((input, result))
    }

    fn parse_array(input: &str) -> IResult<&str, SyscallArg<'_>> {
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

    fn parse_return_value(input: &str) -> IResult<&str, &str> {
        preceded(
            tuple((space0, char('='), space0)),
            map(rest, |s: &str| s.trim()),
        )(input)
    }

    fn parse_pid(input: &str) -> IResult<&str, u32> {
        map(digit1, |s: &str| s.parse::<u32>().unwrap())(input)
    }

    fn parse_none(input: &str) -> IResult<&str, SyscallArg<'_>> {
        // Parse the format like:
        //     func(arg1, , arg3)
        //     func(arg1,)
        value(SyscallArg::Ignored, recognize(peek(char(','))))(input)
    }

    fn parse_signal_line(input: &str) -> IResult<&str, ()> {
        // Parse the format like:
        //     --- SIGTERM {si_signo=SIGTERM, si_code=SI_USER, si_pid=123, si_uid=1000} ---
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("---"), take_until("---"), tag("---")),
            )),
        )(input)
    }

    fn parse_exit_line(input: &str) -> IResult<&str, ()> {
        // Parse the format like:
        //     +++ exited with 0 +++
        value(
            (),
            tuple((
                opt(terminated(Self::parse_pid, space1)),
                delimited(tag("+++"), take_until("+++"), tag("+++")),
            )),
        )(input)
    }

    fn parse_multithread_blocked(input: &str) -> IResult<&str, (u32, String)> {
        // Parse the format like:
        //    123 read(3</path/to/file>, <unfinished ...>
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

    fn parse_multithread_resumed(input: &str) -> IResult<&str, (u32, String)> {
        // Parse the format like:
        //     123 <... read resumed> "\x00\x01\x02", 1024) = 3
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

    /// Handles special cases for certain syscalls whose strace output is non-standard.
    fn handle_special_cases(mut syscall: Syscall) -> Syscall {
        match syscall.name {
            // For `clone`, strace removes the first and fourth arguments, just insert
            // ignored args.
            "clone" => {
                syscall.args.insert(0, SyscallArg::Ignored);
                syscall.args.insert(3, SyscallArg::Ignored);
            }
            _ => {}
        }

        syscall
    }
}
