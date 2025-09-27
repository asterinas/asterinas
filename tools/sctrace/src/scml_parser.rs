// SPDX-License-Identifier: MPL-2.0

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
#[derive(Debug, Clone)]
pub enum ScmlParseError {
    /// File I/O error when reading SCML files.
    IoError(String),
    /// Parsing error with details.
    ParseError(String),
    /// Incomplete statement.
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
#[derive(Debug, Clone, PartialEq)]
pub enum PatternArg {
    /// No constraints - accepts any value.
    None,
    /// Must be an integer type.
    Integer,
    /// Must match a file path pattern.
    Path,
    /// Must match a single flag value.
    Flag(String),
    /// Array of pattern arguments with type consistency.
    Array(PatternArray),
    /// Structured type with named fields and optional wildcard matching.
    Struct(PatternStruct),
    /// Set of flags combinable with bitwise OR operations.
    Flags(PatternFlagSet),
}

/// Array pattern ensuring all elements have the same type.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternArray(Vec<PatternArg>);

/// Set of flags that can be combined using bitwise OR operations.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternFlagSet(Vec<PatternArg>);

/// Structured pattern with named fields and optional wildcard support.
///
/// The boolean field indicates whether the struct accepts additional unspecified fields.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternStruct(HashMap<String, PatternArg>, bool);

impl PatternArray {
    /// Creates a new PatternArray with type consistency validation.
    ///
    /// # Panics
    /// Panics if elements have different types.
    pub fn new(args: Vec<PatternArg>) -> Self {
        if args.is_empty() {
            return Self(args);
        }

        // Ensure all elements have the same type
        let first_discriminant = std::mem::discriminant(&args[0]);
        for arg in &args[1..] {
            if std::mem::discriminant(arg) != first_discriminant {
                panic!("Array elements must be of the same type");
            }
        }

        Self(args)
    }

    /// Returns a reference to the array elements.
    pub fn args(&self) -> &Vec<PatternArg> {
        &self.0
    }
}

impl PatternFlagSet {
    /// Creates a new PatternFlagSet with type validation.
    ///
    /// # Panics
    /// Panics if elements are not Flag or Integer types.
    pub fn new(flags: Vec<PatternArg>) -> Self {
        // Validate that all elements are either Flag or Integer types
        for flag in &flags {
            match flag {
                PatternArg::Flag(_) | PatternArg::Integer => {
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
    pub fn flags(&self) -> &Vec<PatternArg> {
        &self.0
    }
}

impl PatternStruct {
    /// Creates a new PatternStruct.
    ///
    /// # Arguments
    /// * `fields` - Mapping of field names to their pattern constraints
    /// * `wildcard` - Whether to accept additional unspecified fields
    pub fn new(fields: HashMap<String, PatternArg>, wildcard: bool) -> Self {
        Self(fields, wildcard)
    }

    /// Returns a reference to the struct fields.
    pub fn fields(&self) -> &HashMap<String, PatternArg> {
        &self.0
    }

    /// Returns whether this struct pattern accepts wildcard fields.
    pub fn wildcard(&self) -> bool {
        self.1
    }
}

/// Pattern that matches against syscall invocations.
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    /// Name of the pattern (corresponds to syscall name).
    name: String,
    /// Ordered list of argument patterns for this syscall.
    args: Vec<PatternArg>,
}

impl Pattern {
    /// Parses a pattern from preprocessed string input using nom.
    ///
    /// # Input Format
    /// The input should be preprocessed to remove comments, join lines, and expand named references.
    ///
    /// # Parameter Constraints
    /// Parameters can be constrained using `=` or left unconstrained:
    /// - `param = value` - Parameter must match the specified value/pattern
    /// - `param` - Parameter accepts any value
    ///
    /// # Examples
    ///
    /// ## Basic flag constraints:
    /// ```text
    /// open(path, flags = O_CREAT | O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC, mode);
    /// ```
    /// - `path` and `mode` are unconstrained (`PatternArg::None`)
    /// - `flags` must be one or more of the specified flags (`PatternArg::Flags`)
    ///
    /// ## Struct patterns with wildcards:
    /// ```text
    /// sigaction(signum, act = { sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT, .. }, oldact = { sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT, .. });
    /// ```
    /// - `signum` is unconstrained (`PatternArg::None`)
    /// - `act` and `oldact` are structs with wildcard enabled (`PatternArg::Struct`)
    /// - `sa_flags` field contains specified flags (`PatternArg::Flags`)
    ///
    /// ## Array patterns:
    /// ```text
    /// poll(fds = [ { events = POLLIN | POLLOUT | POLLRDHUP | POLLERR | POLLHUP | POLLNVAL, .. } ], nfds, timeout);
    /// ```
    /// - `fds` is an array of structs with wildcard enabled (`PatternArg::Array`)
    /// - Each array element is a struct containing flag constraints
    /// - `nfds` and `timeout` are unconstrained (`PatternArg::None`)
    ///
    /// ## Built-in type constraints:
    /// ```text
    /// read(fd, buf, count = <INTEGER>);
    /// ```
    /// - `fd` and `buf` are unconstrained (`PatternArg::None`)
    /// - `count` must be an integer (`PatternArg::Integer`)
    ///
    /// ## Path constraints:
    /// ```text
    /// openat(dirfd, pathname = <PATH>, flags = O_CREAT | O_RDONLY | O_WRONLY | O_RDWR | O_CLOEXEC, mode);
    /// ```
    /// - `dirfd` and `mode` are unconstrained (`PatternArg::None`)
    /// - `pathname` must match a file path pattern (`PatternArg::Path`)
    /// - `flags` contains specified flag constraints (`PatternArg::Flags`)
    pub fn parse(input: &str) -> IResult<&str, Pattern> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('(')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, args) = opt(Self::parse_param_list)(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = opt(char(','))(input)?; // Allow trailing comma
        let (input, _) = multispace0(input)?;
        let (input, _) = char(')')(input)?;
        let (input, _) = char(';')(input)?;

        let args = args.unwrap_or_default();
        Ok((input, Pattern::new(name.to_string(), args)))
    }

    /// Creates a new pattern with the given name and arguments.
    pub fn new(name: String, args: Vec<PatternArg>) -> Self {
        Self { name, args }
    }

    /// Returns the pattern name (syscall name).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns a reference to the pattern arguments.
    pub fn args(&self) -> &Vec<PatternArg> {
        &self.args
    }

    // Parse parameter list
    fn parse_param_list(input: &str) -> IResult<&str, Vec<PatternArg>> {
        separated_list0(
            delimited(multispace0, char(','), multispace0),
            Self::parse_param,
        )(input)
    }

    // Parse a single parameter
    fn parse_param(input: &str) -> IResult<&str, PatternArg> {
        let (input, _) = multispace0(input)?;
        let (input, _) = identifier(input)?;
        let (input, _) = multispace0(input)?;

        // Check if parameter has constraint (= value)
        if let Ok((input, _)) = char::<&str, nom::error::Error<&str>>('=')(input) {
            let (input, _) = multispace0(input)?;
            Self::parse_expr(input)
        } else {
            // Unconstrained parameter
            Ok((input, PatternArg::None))
        }
    }

    // Parse expression (flags, structs, arrays, etc.)
    fn parse_expr(input: &str) -> IResult<&str, PatternArg> {
        alt((
            Self::parse_struct,
            Self::parse_array,
            Self::parse_builtin_type,
            Self::parse_flags,
        ))(input)
    }

    // Parse struct pattern: { field1 = value1, field2 = value2, .. }
    fn parse_struct(input: &str) -> IResult<&str, PatternArg> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('{')(input)?;
        let (input, _) = multispace0(input)?;

        let (input, fields) = separated_list0(
            delimited(multispace0, char(','), multispace0),
            Self::parse_struct_field,
        )(input)?;

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

    // Parse struct field: field_name = expression or just field_name
    fn parse_struct_field(input: &str) -> IResult<&str, (String, PatternArg)> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;

        if let Ok((input, _)) = char::<&str, nom::error::Error<&str>>('=')(input) {
            let (input, _) = multispace0(input)?;
            let (input, expr) = Self::parse_expr(input)?;
            Ok((input, (name.to_string(), expr)))
        } else {
            Ok((input, (name.to_string(), PatternArg::None)))
        }
    }

    // Parse array pattern: [ expression ]
    fn parse_array(input: &str) -> IResult<&str, PatternArg> {
        let (input, _) = multispace0(input)?;
        let (input, _) = char('[')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, element) = Self::parse_expr(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(']')(input)?;

        Ok((input, PatternArg::Array(PatternArray::new(vec![element]))))
    }

    // Parse flags pattern: FLAG1 | FLAG2 | FLAG3
    fn parse_flags(input: &str) -> IResult<&str, PatternArg> {
        let (input, flags) = separated_list1(
            delimited(multispace0, char('|'), multispace0),
            alt((Self::parse_builtin_type, Self::parse_flag)),
        )(input)?;

        Ok((input, PatternArg::Flags(PatternFlagSet::new(flags))))
    }

    // Parse built-in types: <INTEGER>, <PATH>
    fn parse_builtin_type(input: &str) -> IResult<&str, PatternArg> {
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

    // Parse single flag
    fn parse_flag(input: &str) -> IResult<&str, PatternArg> {
        let (input, _) = multispace0(input)?;
        let (input, flag) = identifier(input)?;
        Ok((input, PatternArg::Flag(flag.to_string())))
    }
}

/// Container for organizing multiple patterns by syscall name.
#[derive(Debug, Clone, PartialEq)]
pub struct Patterns {
    patterns: HashMap<String, Vec<Pattern>>,
}

/// Parses identifiers including digits, alphabetic characters, and underscores.
fn identifier(input: &str) -> IResult<&str, &str> {
    alt((
        nom::character::complete::digit1,
        recognize(pair(
            alt((nom::character::complete::alpha1, tag("_"))),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
    ))(input)
}

impl Patterns {
    /// Creates a new Patterns container.
    pub fn new(patterns: HashMap<String, Vec<Pattern>>) -> Self {
        Self { patterns }
    }

    /// Parses an SCML file and returns a `Patterns` container.
    ///
    /// # Arguments
    /// * `path` - Path to the SCML file
    ///
    /// # Returns
    /// Returns `Ok(Patterns)` on success, or `Err(Box<dyn Error>)` with error description on failure.
    ///
    /// # Errors
    /// - File I/O errors when reading the SCML file
    /// - Parsing errors when encountering malformed SCML syntax
    pub fn from_scml_file(path: &str) -> Result<Self, ScmlParseError> {
        let content = fs::read_to_string(path).map_err(|e| {
            ScmlParseError::IoError(format!("Failed to read file '{}': {}", path, e))
        })?;

        Self::from_scml(&content)
    }

    /// Parses SCML content and returns a `Patterns` container.
    ///
    /// # SCML Language Overview
    ///
    /// SCML (System Call Matching Language) is a domain-specific language for defining
    /// syscall patterns with enhanced pattern matching capabilities. It supports multi-line
    /// definitions and flexible formatting for better readability.
    ///
    /// # Grammar Definition
    ///
    /// ```text
    /// scml           ::= { rule }
    /// rule           ::= syscall-rule ';'
    ///                 | struct-rule ';'
    ///                 | bitflags-rule ';'
    ///
    /// syscall-rule   ::= identifier '(' [ param-list ] ')'
    /// param-list     ::= param { ',' param }
    /// param          ::= identifier '=' expr
    ///                 | identifier
    ///
    /// expr           ::= expr '|' expr
    ///                 | term
    /// term           ::= identifier
    ///                 | '<' identifier '>'
    ///                 | struct
    ///                 | array
    ///
    /// array          ::= '[' expr ']'
    /// struct         ::= '{' field-list [ ',' '..' ] '}'
    /// field-list     ::= field { ',' field }
    /// field          ::= identifier
    ///                 | identifier '=' expr
    ///
    /// struct-rule    ::= 'struct' identifier '=' struct
    /// bitflags-rule  ::= identifier '=' expr
    ///
    /// identifier     ::= ( letter | '_' ) { letter | digit | '_' }
    ///                 | digit+
    ///
    /// comment        ::= '//' { any-char }
    /// ```
    ///
    /// # Language Features
    ///
    /// ## Comments
    /// C-style line comments using `//` - all text after `//` is ignored.
    ///
    /// ## Named Bitflags
    /// Define reusable flag sets that can be referenced across rules:
    ///
    /// ```text
    /// access_mode = O_RDONLY | O_WRONLY | O_RDWR;
    ///
    /// open(path, flags = O_CREAT | <access_mode>);
    /// ```
    ///
    /// The parser expands named references, resulting in a `PatternFlagSet` containing
    /// `O_CREAT`, `O_RDONLY`, `O_WRONLY`, and `O_RDWR`. Later definitions override earlier ones.
    ///
    /// ## Struct Patterns
    /// Define constraints on structured data with optional wildcard matching:
    ///
    /// ```text
    /// sigaction(signum,
    ///           act = {
    ///               sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT,
    ///               ..
    ///           },
    ///           oldact = {
    ///               sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT,
    ///               ..
    ///           }
    /// );
    /// ```
    ///
    /// - Fields are mapped to their constraints
    /// - `..` enables wildcard matching for unspecified fields
    /// - Results in `PatternStruct` with field constraints and wildcard flag
    ///
    /// ## Named Structs
    /// Define reusable struct patterns with flexible definition rules:
    ///
    /// ```text
    /// struct sigaction = {
    ///     sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT,
    ///     ..
    /// };
    ///
    /// sigaction(signum, act = <sigaction>, oldact = <sigaction>);
    /// ```
    ///
    /// **Definition Rules:**
    /// - **Continuous definitions:** Multiple consecutive struct definitions with the same name
    ///   create multiple patterns for that struct
    /// - **Discontinuous definitions:** If a struct is redefined after other statements,
    ///   the new definition replaces all previous definitions
    ///
    /// ## Array Patterns
    /// Define constraints on array elements with type consistency:
    ///
    /// ```text
    /// events = POLLIN | POLLOUT | POLLRDHUP | POLLERR | POLLHUP | POLLNVAL;
    /// struct pollfd = {
    ///     events = <events>,
    ///     revents = <events>,
    ///     ..
    /// };
    ///
    /// poll(fds = [ <pollfd> ], nfds, timeout);
    /// ```
    ///
    /// - `fds` is an array where each element must match the `pollfd` struct pattern
    /// - Results in `PatternArray` containing `PatternStruct` for `pollfd`
    ///
    /// ## Built-in Types
    /// Special built-in type constraints:
    /// - `<INTEGER>` - Constrains parameter to integer values (`PatternArg::Integer`)
    /// - `<PATH>` - Constrains parameter to file path patterns (`PatternArg::Path`)
    ///
    /// # Parsing Process
    /// 1. **Preprocessing:** Remove comments and join lines until semicolons
    /// 2. **Named definitions:** Store bitflags and struct definitions in maps,
    ///    handling redefinition rules
    /// 3. **Reference expansion:** Replace named references in syscall rules
    ///    using stored definitions
    /// 4. **Pattern building:** Create `Pattern` objects with appropriate `PatternArg` constraints
    /// 5. **Organization:** Group patterns by syscall name in the result `HashMap`
    ///
    /// # Arguments
    /// * `content` - SCML content as a string
    ///
    /// # Returns
    /// Returns `Ok(Patterns)` on success, or `Err(Box<dyn Error>)` with error description on failure.
    ///
    /// # Errors
    /// - Parsing errors when encountering malformed SCML syntax
    /// - Pattern construction errors due to invalid constraints
    pub fn from_scml(content: &str) -> Result<Self, ScmlParseError> {
        let statements = Self::preprocess_content(content)?;
        let mut patterns: HashMap<String, Vec<Pattern>> = HashMap::new();
        let mut errors = Vec::new();

        for statement in statements {
            match Pattern::parse(&statement) {
                Ok((remaining, pattern)) => {
                    if !remaining.trim().is_empty() {
                        errors.push(format!(
                            "Warning: Unparsed input remaining in statement '{}': '{}'",
                            statement, remaining
                        ));
                    }
                    patterns
                        .entry(pattern.name().to_string())
                        .or_default()
                        .push(pattern);
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

        Ok(Self::new(patterns))
    }

    /// Preprocesses SCML content by removing comments, joining lines, and expanding named references.
    ///
    /// # Returns
    /// Vector of complete statements ready for parsing.
    ///
    /// # Errors
    /// Returns an error if there is an incomplete statement at the end of the content.
    fn preprocess_content(content: &str) -> Result<Vec<String>, ScmlParseError> {
        let content = Self::remove_comments(content)?;
        Self::replace_named_references(content)
    }

    /// Removes comments and joins lines until semicolons.
    ///
    /// # Processing Rules
    /// - Remove lines starting with `//` or empty lines
    /// - Join lines until a semicolon is found
    /// - Trim whitespace appropriately
    ///
    /// # Returns
    /// Vector of complete statements.
    ///
    /// # Errors
    /// Returns an error if there is an incomplete statement at the end of the content.
    fn remove_comments(content: &str) -> Result<Vec<String>, ScmlParseError> {
        let mut statements = Vec::new();
        let mut current_statement = String::new();

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("//") || line.is_empty() {
                continue;
            }
            let line = line.trim_end();

            current_statement.push_str(line);
            current_statement.push(' ');

            if current_statement.trim().ends_with(';') {
                statements.push(current_statement.trim().to_string());
                current_statement.clear();
            }
        }

        if !current_statement.trim().is_empty() {
            return Err(ScmlParseError::IncompleteStatement(
                current_statement.trim().to_string(),
            ));
        }

        Ok(statements)
    }

    /// Parses and replaces named references in the content.
    ///
    /// # Named Reference Rules
    /// - **Bitflags:** Latest definition replaces previous ones
    /// - **Structs:**
    ///   - Continuous definitions create multiple patterns
    ///   - Discontinuous definitions replace all previous ones
    ///
    /// # Returns
    /// Vector of statements with named references expanded.
    fn replace_named_references(statements: Vec<String>) -> Result<Vec<String>, ScmlParseError> {
        let mut named_bitflags: HashMap<String, String> = HashMap::new();
        let mut named_structs: HashMap<String, Vec<String>> = HashMap::new();
        let mut last_struct_name: Option<String> = None;
        let mut result = Vec::new();

        for statement in statements {
            let trimmed = statement.trim();

            // Handle bitflags definition
            if let Ok((_, (name, flags))) = Self::parse_bitflags_definition(trimmed) {
                let expanded_flags =
                    Self::expand_named_references(&flags, &named_bitflags, &named_structs);
                for expanded_named_flags in expanded_flags {
                    named_bitflags.insert(name.clone(), expanded_named_flags);
                }
                last_struct_name = None;

                continue;
            }

            // Handle struct definition
            if let Ok((_, (name, struct_def))) = Self::parse_struct_definition(trimmed) {
                let expanded_struct_def =
                    Self::expand_named_references(&struct_def, &named_bitflags, &named_structs);

                for expanded_named_struct_def in expanded_struct_def {
                    if let Some(ref last_name) = last_struct_name {
                        if last_name == &name {
                            // Continuous definition
                            if let Some(definitions) = named_structs.get_mut(&name) {
                                definitions.push(expanded_named_struct_def);
                            }
                        } else {
                            // Different struct name
                            named_structs.insert(name.clone(), vec![expanded_named_struct_def]);
                        }
                    } else {
                        // First struct or after non-struct statement
                        named_structs.insert(name.clone(), vec![expanded_named_struct_def]);
                    }

                    last_struct_name = Some(name.clone());
                }

                continue;
            }

            // Handle syscall rule
            last_struct_name = None;
            let expanded_statements =
                Self::expand_named_references(&statement, &named_bitflags, &named_structs);
            result.extend(expanded_statements);
        }

        Ok(result)
    }

    /// Parse bitflags definition like: "access_mode = O_RDONLY | O_WRONLY | O_RDWR;"
    fn parse_bitflags_definition(input: &str) -> IResult<&str, (String, String)> {
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('=')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, flags) = Self::take_until_semicolon(input)?;
        let (input, _) = char(';')(input)?;

        Ok((input, (name.to_string(), flags.trim().to_string())))
    }

    /// Parse struct definition like: "struct sigaction = { sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT, .. };"
    fn parse_struct_definition(input: &str) -> IResult<&str, (String, String)> {
        let (input, _) = multispace0(input)?;
        let (input, _) = tag("struct")(input)?;
        let (input, _) = multispace0(input)?;
        let (input, name) = identifier(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char('=')(input)?;
        let (input, _) = multispace0(input)?;
        let (input, struct_body) =
            delimited(char('{'), Self::take_until_closing_brace, char('}'))(input)?;
        let (input, _) = multispace0(input)?;
        let (input, _) = char(';')(input)?;

        Ok((input, (name.to_string(), struct_body.trim().to_string())))
    }

    // Helper function to take content until semicolon
    fn take_until_semicolon(input: &str) -> IResult<&str, &str> {
        let mut chars = input.char_indices();
        while let Some((i, c)) = chars.next() {
            if c == ';' {
                return Ok((&input[i..], &input[..i]));
            }
        }
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }

    // Helper function to take content until closing brace
    fn take_until_closing_brace(input: &str) -> IResult<&str, &str> {
        let mut chars = input.char_indices();
        let mut brace_count = 0;

        while let Some((i, c)) = chars.next() {
            match c {
                '{' => brace_count += 1,
                '}' => {
                    if brace_count == 0 {
                        return Ok((&input[i..], &input[..i]));
                    }
                    brace_count -= 1;
                }
                _ => {}
            }
        }
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }

    // Unified function to expand named references
    fn expand_named_references(
        input: &str,
        named_bitflags: &HashMap<String, String>,
        named_structs: &HashMap<String, Vec<String>>,
    ) -> Vec<String> {
        let mut statements = vec![input.to_string()];

        // Expand bitflags references like <access_mode>
        for (name, definition) in named_bitflags {
            let reference = format!("<{}>", name);
            for stmt in &mut statements {
                if stmt.contains(&reference) {
                    *stmt = stmt.replace(&reference, definition);
                }
            }
        }

        // Expand struct references like <sigaction>
        // For syscall rules: generate multiple statements for each struct definition
        for (name, definitions) in named_structs {
            let reference = format!("<{}>", name);
            let mut new_statements = Vec::new();

            for stmt in &statements {
                if stmt.contains(&reference) {
                    // Generate one statement for each struct definition
                    for definition in definitions {
                        let expanded = format!("{{ {} }}", definition);
                        let new_stmt = stmt.replace(&reference, &expanded);
                        new_statements.push(new_stmt);
                    }
                } else {
                    new_statements.push(stmt.clone());
                }
            }

            // Only update statements if we found any references to replace
            if new_statements.len() != statements.len()
                || new_statements
                    .iter()
                    .zip(statements.iter())
                    .any(|(a, b)| a != b)
            {
                statements = new_statements;
            }
        }

        statements
    }

    /// Retrieves all patterns for a specific syscall name.
    ///
    /// # Arguments
    /// * `name` - The syscall name to look up
    ///
    /// # Returns
    /// `Some(&Vec<Pattern>)` if patterns exist for the syscall, `None` otherwise.
    pub fn get(&self, name: &str) -> Option<&Vec<Pattern>> {
        self.patterns.get(name)
    }

    /// Returns the total number of unique syscall patterns stored.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_scml_basic_syscall() {
        let content = "read(fd, buf, count);";
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("read").unwrap();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].name(), "read");
        assert_eq!(patterns[0].args().len(), 3);

        // All parameters should be unconstrained
        for arg in patterns[0].args() {
            matches!(arg, PatternArg::None);
        }
    }

    #[test]
    fn test_from_scml_constrained_parameters() {
        let content = "open(path, flags = O_CREAT | O_RDONLY, mode = <INTEGER>);";
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("open").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // path should be unconstrained
        matches!(pattern.args()[0], PatternArg::None);

        // flags should be a flag set
        if let PatternArg::Flags(flag_set) = &pattern.args()[1] {
            assert_eq!(flag_set.flags().len(), 2);
        } else {
            panic!("Expected PatternArg::Flags");
        }

        // mode should be integer
        matches!(pattern.args()[2], PatternArg::Integer);
    }

    #[test]
    fn test_from_scml_struct_pattern() {
        let content = r#"
        sigaction(signum,
                  act = { sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT, .. },
                  oldact = { sa_flags = SA_RESTART, .. });
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("sigaction").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // signum should be unconstrained
        matches!(pattern.args()[0], PatternArg::None);

        // act should be a struct with wildcard
        if let PatternArg::Struct(struct_pattern) = &pattern.args()[1] {
            assert!(struct_pattern.wildcard());
            assert_eq!(struct_pattern.fields().len(), 1);
            assert!(struct_pattern.fields().contains_key("sa_flags"));
        } else {
            panic!("Expected PatternArg::Struct");
        }

        // oldact should also be a struct with wildcard
        if let PatternArg::Struct(struct_pattern) = &pattern.args()[2] {
            assert!(struct_pattern.wildcard());
            assert_eq!(struct_pattern.fields().len(), 1);
            assert!(struct_pattern.fields().contains_key("sa_flags"));
        } else {
            panic!("Expected PatternArg::Struct");
        }
    }

    #[test]
    fn test_from_scml_array_pattern() {
        let content = r#"
        poll(fds = [ { events = POLLIN | POLLOUT, revents = POLLERR, .. } ],
             nfds,
             timeout);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("poll").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // fds should be an array
        if let PatternArg::Array(array_pattern) = &pattern.args()[0] {
            assert_eq!(array_pattern.args().len(), 1);
            if let PatternArg::Struct(struct_pattern) = &array_pattern.args()[0] {
                assert!(struct_pattern.wildcard());
                assert_eq!(struct_pattern.fields().len(), 2);
                assert!(struct_pattern.fields().contains_key("events"));
                assert!(struct_pattern.fields().contains_key("revents"));
            } else {
                panic!("Expected PatternArg::Struct in array");
            }
        } else {
            panic!("Expected PatternArg::Array");
        }

        // nfds and timeout should be unconstrained
        matches!(pattern.args()[1], PatternArg::None);
        matches!(pattern.args()[2], PatternArg::None);
    }

    #[test]
    fn test_from_scml_builtin_types() {
        let content = r#"
        openat(dirfd, pathname = <PATH>, flags = O_RDONLY, mode = <INTEGER>);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("openat").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // dirfd should be unconstrained
        matches!(pattern.args()[0], PatternArg::None);

        // pathname should be PATH type
        matches!(pattern.args()[1], PatternArg::Path);

        // flags should be a single flag
        if let PatternArg::Flags(flag_set) = &pattern.args()[2] {
            assert_eq!(flag_set.flags().len(), 1);
            if let PatternArg::Flag(flag) = &flag_set.flags()[0] {
                assert_eq!(flag, "O_RDONLY");
            } else {
                panic!("Expected PatternArg::Flag inside Flags");
            }
        } else {
            panic!("Expected PatternArg::Flag");
        }

        // mode should be integer
        matches!(pattern.args()[3], PatternArg::Integer);
    }

    #[test]
    fn test_from_scml_named_bitflags() {
        let content = r#"
        access_mode = O_RDONLY | O_WRONLY | O_RDWR;
        open(path, flags = O_CREAT | <access_mode>, mode);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("open").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // flags should contain all expanded flags
        if let PatternArg::Flags(flag_set) = &pattern.args()[1] {
            assert_eq!(flag_set.flags().len(), 4); // O_CREAT + 3 access modes
        } else {
            panic!("Expected PatternArg::Flags");
        }
    }

    #[test]
    fn test_from_scml_named_structs() {
        let content = r#"
        struct sigaction = {
            sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT,
            sa_handler,
            ..
        };

        sigaction(signum, act = <sigaction>, oldact = <sigaction>);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("sigaction").unwrap();
        assert_eq!(patterns.len(), 1);
        let pattern = &patterns[0];

        // Both act and oldact should be structs with the defined fields
        for i in 1..3 {
            if let PatternArg::Struct(struct_pattern) = &pattern.args()[i] {
                assert!(struct_pattern.wildcard());
                assert_eq!(struct_pattern.fields().len(), 2);
                assert!(struct_pattern.fields().contains_key("sa_flags"));
                assert!(struct_pattern.fields().contains_key("sa_handler"));
            } else {
                panic!("Expected PatternArg::Struct");
            }
        }
    }

    #[test]
    fn test_from_scml_without_parameters() {
        let content = "getuid();";
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("getuid").unwrap();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].args().len(), 0); // No parameters
    }

    #[test]
    fn test_from_scml_multiple_struct_definitions() {
        let content = r#"
        struct pollfd = { fd, events = POLLIN };
        struct pollfd = { fd, events = POLLOUT };

        poll(fds = [ <pollfd> ], nfds, timeout);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("poll").unwrap();
        assert_eq!(patterns.len(), 2); // Should generate 2 patterns for 2 struct definitions
    }

    #[test]
    fn test_from_scml_comments_and_whitespace() {
        let content = r#"
        // This is a comment
        open(path,
             flags = O_CREAT | O_RDONLY,
             mode);

        // Another syscall
        read(fd, buf, count);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        assert!(result.get("open").is_some());
        assert!(result.get("read").is_some());

        let open_patterns = result.get("open").unwrap();
        assert_eq!(open_patterns.len(), 1);
        assert_eq!(open_patterns[0].args().len(), 3);
    }

    #[test]
    fn test_from_scml_trailing_comma() {
        let content = r#"
        open(path, flags = O_CREAT, mode,);
        read(fd, buf, count,);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        assert!(result.get("open").is_some());
        assert!(result.get("read").is_some());
    }

    #[test]
    fn test_from_scml_empty_struct() {
        let content = "test(arg = {});";
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("test").unwrap();
        assert_eq!(patterns.len(), 1);

        if let PatternArg::Struct(struct_pattern) = &patterns[0].args()[0] {
            assert!(!struct_pattern.wildcard());
            assert_eq!(struct_pattern.fields().len(), 0);
        } else {
            panic!("Expected PatternArg::Struct");
        }
    }

    #[test]
    fn test_from_scml_nested_structures() {
        let content = r#"
        test(arg = {
            inner = {
                flags = FLAG1 | FLAG2,
                ..
            },
            array = [ { value = <INTEGER> } ],
            ..
        });
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("test").unwrap();
        assert_eq!(patterns.len(), 1);

        if let PatternArg::Struct(struct_pattern) = &patterns[0].args()[0] {
            assert!(struct_pattern.wildcard());
            assert_eq!(struct_pattern.fields().len(), 2);
        } else {
            panic!("Expected PatternArg::Struct");
        }
    }

    #[test]
    fn test_from_scml_error_handling_invalid_syntax() {
        let content = "invalid syntax without semicolon";
        let result = Patterns::from_scml(content);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScmlParseError::IncompleteStatement(_) => {
                // Expected error type
            }
            _ => panic!("Expected IncompleteStatement error"),
        }
    }

    #[test]
    fn test_from_scml_error_handling_malformed_struct() {
        let content = "test(arg = { missing_closing_brace );";
        let result = Patterns::from_scml(content);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScmlParseError::ParseError(_) => {
                // Expected error type
            }
            _ => panic!("Expected Parsing error"),
        }
    }

    #[test]
    fn test_from_scml_error_handling_malformed_array() {
        let content = "test(arg = [ missing_closing_bracket );";
        let result = Patterns::from_scml(content);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScmlParseError::ParseError(_) => {
                // Expected error type
            }
            _ => panic!("Expected Parsing error"),
        }
    }

    #[test]
    fn test_from_scml_error_handling_invalid_builtin_type() {
        let content = "test(arg = <INVALID_TYPE>);";
        let result = Patterns::from_scml(content);
        assert!(result.is_err());
        match result.unwrap_err() {
            ScmlParseError::ParseError(_) => {
                // Expected error type
            }
            _ => panic!("Expected Parsing error"),
        }
    }

    #[test]
    fn test_from_scml_multiple_syscalls_same_name() {
        let content = r#"
        open(path, flags = O_RDONLY);
        open(path, flags = O_WRONLY);
        open(path, flags = O_RDWR);
        "#;
        let result = Patterns::from_scml(content).unwrap();

        let patterns = result.get("open").unwrap();
        assert_eq!(patterns.len(), 3); // Should have 3 different patterns for open
    }

    #[test]
    fn test_from_scml_complex_real_world_example() {
        let content = r#"
        // Define common flag sets
        access_mode = O_RDONLY | O_WRONLY | O_RDWR;
        open_flags = O_CREAT | O_EXCL | O_TRUNC | O_APPEND | O_CLOEXEC;

        // Define reusable struct patterns
        struct stat = {
            st_mode = <INTEGER>,
            st_size = <INTEGER>,
            ..
        };

        struct sigaction = {
            sa_flags = SA_NOCLDSTOP | SA_NOCLDWAIT | SA_RESTART,
            ..
        };

        // System call patterns
        open(pathname = <PATH>, flags = <access_mode> | <open_flags>, mode = <INTEGER>);
        read(fd, buf, count = <INTEGER>);
        write(fd, buf, count = <INTEGER>);
        stat(pathname = <PATH>, statbuf = stat);
        sigaction(signum, act = <sigaction>, oldact = <sigaction>);

        // Complex pattern with arrays and nested structures
        poll(fds = [ {
            fd,
            events = POLLIN | POLLOUT | POLLRDHUP | POLLERR | POLLHUP | POLLNVAL,
            revents = POLLIN | POLLOUT | POLLRDHUP | POLLERR | POLLHUP | POLLNVAL,
            ..
        } ], nfds = <INTEGER>, timeout = <INTEGER>);
        "#;

        let result = Patterns::from_scml(content).unwrap();

        // Verify all syscalls were parsed
        assert!(result.get("open").is_some());
        assert!(result.get("read").is_some());
        assert!(result.get("write").is_some());
        assert!(result.get("stat").is_some());
        assert!(result.get("sigaction").is_some());
        assert!(result.get("poll").is_some());

        // Verify open has expanded flags
        let open_patterns = result.get("open").unwrap();
        assert_eq!(open_patterns.len(), 1);
        if let PatternArg::Flags(flag_set) = &open_patterns[0].args()[1] {
            assert!(flag_set.flags().len() >= 8); // access_mode (3) + open_flags (5)
        } else {
            panic!("Expected PatternArg::Flags for open flags");
        }

        // Verify poll has complex array structure
        let poll_patterns = result.get("poll").unwrap();
        assert_eq!(poll_patterns.len(), 1);
        if let PatternArg::Array(array_pattern) = &poll_patterns[0].args()[0] {
            if let PatternArg::Struct(struct_pattern) = &array_pattern.args()[0] {
                assert!(struct_pattern.wildcard());
                assert_eq!(struct_pattern.fields().len(), 3);
            } else {
                panic!("Expected PatternArg::Struct in poll array");
            }
        } else {
            panic!("Expected PatternArg::Array for poll fds");
        }
    }

    #[test]
    fn test_from_scml_advanced_usage_for_same_struct() {
        let content = r#"
        // Rules for control message header
        struct cmsghdr = {
            cmsg_level = SOL_SOCKET,
            cmsg_type  = SO_TIMESTAMP_OLD | SCM_RIGHTS | SCM_CREDENTIALS,
            ..
        };
        struct cmsghdr = {
            cmsg_level = SOL_IP,
            cmsg_type  = IP_TTL,
            ..
        };

        // Rule for message header, which refers to the rules for control message header
        struct msghdr = {
            msg_control = [ <cmsghdr> ],
            ..
        };

        recvmsg(socket, message = <msghdr>, flags);
        "#;

        let result = Patterns::from_scml(content).unwrap();

        // Verify all syscalls were parsed
        assert!(result.get("recvmsg").is_some());

        // Verify recvmsg has expanded flags
        let recvmsg_patterns = result.get("recvmsg").unwrap();
        assert_eq!(recvmsg_patterns.len(), 2);

        assert_eq!(
            recvmsg_patterns[0],
            Pattern::new(
                "recvmsg".to_string(),
                vec![
                    PatternArg::None,
                    PatternArg::Struct(PatternStruct::new(
                        {
                            let mut fields = HashMap::new();
                            fields.insert(
                                "msg_control".to_string(),
                                PatternArg::Array(PatternArray::new(vec![PatternArg::Struct(
                                    PatternStruct::new(
                                        {
                                            let mut fields = HashMap::new();
                                            fields.insert(
                                                "cmsg_level".to_string(),
                                                PatternArg::Flags(PatternFlagSet::new(vec![
                                                    PatternArg::Flag("SOL_SOCKET".to_string()),
                                                ])),
                                            );
                                            fields.insert(
                                                "cmsg_type".to_string(),
                                                PatternArg::Flags(PatternFlagSet::new(vec![
                                                    PatternArg::Flag(
                                                        "SO_TIMESTAMP_OLD".to_string(),
                                                    ),
                                                    PatternArg::Flag("SCM_RIGHTS".to_string()),
                                                    PatternArg::Flag("SCM_CREDENTIALS".to_string()),
                                                ])),
                                            );
                                            fields
                                        },
                                        true,
                                    ),
                                )])),
                            );
                            fields
                        },
                        true
                    )),
                    PatternArg::None,
                ]
            )
        );

        assert_eq!(
            recvmsg_patterns[1],
            Pattern::new(
                "recvmsg".to_string(),
                vec![
                    PatternArg::None,
                    PatternArg::Struct(PatternStruct::new(
                        {
                            let mut fields = HashMap::new();
                            fields.insert(
                                "msg_control".to_string(),
                                PatternArg::Array(PatternArray::new(vec![PatternArg::Struct(
                                    PatternStruct::new(
                                        {
                                            let mut fields = HashMap::new();
                                            fields.insert(
                                                "cmsg_level".to_string(),
                                                PatternArg::Flags(PatternFlagSet::new(vec![
                                                    PatternArg::Flag("SOL_IP".to_string()),
                                                ])),
                                            );
                                            fields.insert(
                                                "cmsg_type".to_string(),
                                                PatternArg::Flags(PatternFlagSet::new(vec![
                                                    PatternArg::Flag("IP_TTL".to_string()),
                                                ])),
                                            );
                                            fields
                                        },
                                        true,
                                    ),
                                )])),
                            );
                            fields
                        },
                        true
                    )),
                    PatternArg::None,
                ]
            )
        );
    }

    #[test]
    fn test_from_scml_empty_content() {
        let content = "";
        let result = Patterns::from_scml(content).unwrap();
        assert!(result.get("any").is_none());
    }

    #[test]
    fn test_from_scml_only_comments() {
        let content = r#"
        // This is just a comment
        // Another comment

        // More comments
        "#;
        let result = Patterns::from_scml(content).unwrap();
        assert!(result.get("any").is_none());
    }
}
