// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashMap, error::Error, fmt, fs, path::Path};

use nom::{
    IResult,
    branch::alt,
    bytes::complete::{tag, take_while},
    character::complete::{char, multispace0},
    combinator::{opt, recognize},
    multi::{separated_list0, separated_list1},
    sequence::{delimited, pair, preceded},
};

#[derive(Debug, Clone)]
pub enum ScmlParseError {
    IoError(String),
    ParseError(String),
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

impl From<ScmlParseError> for String {
    fn from(err: ScmlParseError) -> Self {
        err.to_string()
    }
}

impl Error for ScmlParseError {}

/// Collection of syscall patterns parsed from SCML files.
#[derive(Debug, Clone, PartialEq)]
pub struct Patterns<'a> {
    /// Map from syscall names to their associated patterns.
    patterns: HashMap<&'a str, Vec<Pattern<'a>>>,

    /// Parser context with variable definitions.
    ctx: ParserCtx<'a>,
}

impl<'a> Patterns<'a> {
    /// Reads SCML file(s) and parses all pattern and variable definitions.
    pub fn from_scml_files<P: AsRef<Path>>(paths: &Vec<P>) -> Result<Self, ScmlParseError> {
        let mut all_patterns = Patterns::default();

        for path in paths {
            let patterns = {
                let content = fs::read_to_string(path).map_err(|e| {
                    ScmlParseError::IoError(format!(
                        "Failed to read file '{}': {}",
                        path.as_ref().display(),
                        e
                    ))
                })?;
                // Using the file path as variable prefix to avoid name clashes
                Self::from_scml_with_var_prefix(path.as_ref().display().to_string(), &content)?
            };

            all_patterns.merge(patterns);
        }

        Ok(all_patterns)
    }

    /// Parses SCML content into patterns.
    pub fn from_scml(content: &str) -> Result<Self, ScmlParseError> {
        Self::from_scml_with_var_prefix("".to_string(), content)
    }

    /// Retrieves all patterns for a specific syscall name.
    pub(crate) fn get(&self, name: &str) -> Option<&Vec<Pattern<'_>>> {
        self.patterns.get(name)
    }

    /// Returns the parser context with variable definitions.
    pub(crate) fn ctx(&self) -> &ParserCtx<'_> {
        &self.ctx
    }

    fn new(patterns: HashMap<&'a str, Vec<Pattern<'a>>>, ctx: ParserCtx<'a>) -> Self {
        Self { patterns, ctx }
    }
}

impl Default for Patterns<'_> {
    fn default() -> Self {
        Self::new(HashMap::new(), ParserCtx::new())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pattern<'a> {
    /// Name of the pattern (corresponds to syscall name).
    name: &'a str,

    /// Ordered list of argument patterns for this syscall.
    args: Vec<PatternArg<'a>>,

    /// Whether this pattern accepts additional unspecified arguments.
    wildcard: bool,
}

impl<'a> Pattern<'a> {
    pub(crate) fn name(&self) -> &'a str {
        self.name
    }

    pub(crate) fn args(&self) -> &Vec<PatternArg<'a>> {
        &self.args
    }

    pub(crate) fn wildcard(&self) -> bool {
        self.wildcard
    }

    fn new(name: &'a str, args: Vec<PatternArg<'a>>, wildcard: bool) -> Self {
        Self {
            name,
            args,
            wildcard,
        }
    }

    fn parse(ctx: &ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, Pattern<'a>> {
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

    fn parse_definition(ctx: &mut ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, ()> {
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
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PatternArg<'a> {
    /// Matches all values (no constraint).
    None,

    /// Matches integer arguments.
    Integer,

    /// Matches path arguments.
    Path,

    /// Matches specific flag values like `O_RDONLY`.
    Flag(&'a str),

    /// Matches array arguments where each element matches the corresponding
    /// pattern in the array.
    Array(PatternArray<'a>),

    /// Matches struct arguments with specified field constraints.
    Struct(PatternStruct<'a>),

    /// Matches if any of the struct patterns match. Used when a struct
    /// variable is defined multiple times with different field combinations.
    MultipleStruct(PatternMultipleStruct<'a>),

    /// Matches flag combinations like `O_RDWR | O_CREAT`.
    Flags(PatternFlagSet<'a>),

    /// Contains the variable ID that resolves to a `PatternFlagSet`.
    FlagsVariable(&'a str),

    /// Contains the variable ID that resolves to a `PatternStruct`
    /// or `PatternMultipleStruct`.
    StructVariable(&'a str),
}

impl PatternArg<'_> {
    /// Dereferences `FlagsVariable` and `StructVariable` types to
    /// retrieve their actual pattern definitions from the parser context.
    pub(crate) fn get<'b>(&self, ctx: &'b ParserCtx) -> &'b PatternArg<'b> {
        match self {
            PatternArg::FlagsVariable(id) => ctx.flags_lookup(id).unwrap(),

            PatternArg::StructVariable(id) => ctx.struct_lookup(id).unwrap(),

            _ => {
                panic!("get() can only be called on variable reference types");
            }
        }
    }
}

/// An array pattern matches array arguments where each element must match
/// one of the specified patterns in the array.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatternArray<'a>(Vec<PatternArg<'a>>);

impl<'a> PatternArray<'a> {
    pub(crate) fn args(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }

    fn new(args: Vec<PatternArg<'a>>) -> Self {
        Self(args)
    }
}

/// A flag set pattern matches bitwise OR combinations of flags.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatternFlagSet<'a>(Vec<PatternArg<'a>>);

impl<'a> PatternFlagSet<'a> {
    pub(crate) fn flags(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }

    fn new(flags: Vec<PatternArg<'a>>) -> Self {
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
}

/// A struct pattern matches structured data with named fields. It can specify
/// exact field matching or allow additional fields via wildcard.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatternStruct<'a> {
    fields: HashMap<&'a str, PatternArg<'a>>,
    wildcard: bool,
}

impl<'a> PatternStruct<'a> {
    pub(crate) fn fields(&self) -> &HashMap<&'a str, PatternArg<'a>> {
        &self.fields
    }

    pub(crate) fn wildcard(&self) -> bool {
        self.wildcard
    }

    fn new(fields: HashMap<&'a str, PatternArg<'a>>, wildcard: bool) -> Self {
        Self { fields, wildcard }
    }
}

/// A multiple struct pattern allows defining a struct variable multiple times
/// with different field combinations. The pattern matches if any of the alternatives
/// match.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatternMultipleStruct<'a>(Vec<PatternArg<'a>>);

impl<'a> PatternMultipleStruct<'a> {
    pub(crate) fn structs(&self) -> &Vec<PatternArg<'a>> {
        &self.0
    }

    fn new(structs: Vec<PatternArg<'a>>) -> Self {
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
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParserCtx<'a> {
    /// Map of flags variable IDs to their pattern definitions.
    flags_variables: HashMap<&'a str, PatternArg<'a>>,
    /// Map of struct variable IDs to their pattern definitions.
    struct_variables: HashMap<&'a str, PatternArg<'a>>,
    /// Map of multiple struct variable IDs to their pattern definitions.
    multiple_struct_variables: HashMap<&'a str, PatternArg<'a>>,
}

impl<'a> ParserCtx<'a> {
    fn new() -> Self {
        Self {
            flags_variables: HashMap::new(),
            struct_variables: HashMap::new(),
            multiple_struct_variables: HashMap::new(),
        }
    }

    fn flags_lookup(&self, id: &str) -> Option<&PatternArg<'a>> {
        self.flags_variables.get(id)
    }

    fn struct_lookup(&self, id: &str) -> Option<&PatternArg<'a>> {
        if let Some(pattern) = self.struct_variables.get(id) {
            Some(pattern)
        } else if let Some(pattern) = self.multiple_struct_variables.get(id) {
            Some(pattern)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ParserCtxBuilder<'a> {
    variable_prefix: String,
    /// The parser context being built.
    parser_ctx: ParserCtx<'a>,
    /// Map of named bitflags to their internal IDs.
    named_bitflags: HashMap<&'a str, &'a str>,
    /// Map of named structs to their internal IDs.
    named_structs: HashMap<&'a str, &'a str>,
    /// Last parsed struct definition.
    last_struct: Option<(&'a str, &'a str)>,
    /// Counter for generating unique variable IDs.
    counter: usize,
}

impl<'a> ParserCtxBuilder<'a> {
    fn new(name: String) -> Self {
        Self {
            variable_prefix: name,
            parser_ctx: ParserCtx::new(),
            named_bitflags: HashMap::new(),
            named_structs: HashMap::new(),
            last_struct: None,
            counter: 0,
        }
    }

    fn build(self) -> ParserCtx<'a> {
        self.parser_ctx
    }

    fn generate_variable_id(&mut self) -> &'static str {
        self.counter += 1;
        Box::leak(Box::new(format!(
            "{}/var_{}",
            self.variable_prefix, self.counter
        )))
    }

    fn flags_variables_mut(&mut self) -> &mut HashMap<&'a str, PatternArg<'a>> {
        &mut self.parser_ctx.flags_variables
    }

    fn struct_variables_mut(&mut self) -> &mut HashMap<&'a str, PatternArg<'a>> {
        &mut self.parser_ctx.struct_variables
    }

    fn multiple_struct_variables_mut(&mut self) -> &mut HashMap<&'a str, PatternArg<'a>> {
        &mut self.parser_ctx.multiple_struct_variables
    }

    fn insert_named_bitflag(&mut self, name: &'a str, id: &'a str) {
        self.named_bitflags.insert(name, id);
    }

    fn insert_named_struct(&mut self, name: &'a str, id: &'a str) {
        self.named_structs.insert(name, id);
    }

    fn add_flags_variable(&mut self, id: &'a str, flags: PatternArg<'a>) {
        self.flags_variables_mut().insert(id, flags);
    }

    fn add_struct_variable(&mut self, id: &'a str, struct_def: PatternArg<'a>) {
        self.struct_variables_mut().insert(id, struct_def);
    }

    fn add_multiple_struct_variable(&mut self, id: &'a str, struct_def: PatternArg<'a>) {
        if self.multiple_struct_variables_mut().contains_key(id) {
            self.append_to_multiple_struct(id, struct_def);
        } else {
            self.convert_struct_to_multiple(id);
            self.append_to_multiple_struct(id, struct_def);
        }
    }

    fn convert_struct_to_multiple(&mut self, id: &'a str) {
        if let Some(existing_struct) = self.struct_variables_mut().remove(id) {
            let multiple_struct =
                PatternArg::MultipleStruct(PatternMultipleStruct::new(vec![existing_struct]));
            self.multiple_struct_variables_mut()
                .insert(id, multiple_struct);
        } else {
            panic!("Struct variable should exist for conversion");
        }
    }

    fn append_to_multiple_struct(&mut self, id: &str, struct_def: PatternArg<'a>) {
        let multiple_struct = self
            .parser_ctx
            .multiple_struct_variables
            .get_mut(id)
            .expect("Multiple struct variable should exist");

        if let PatternArg::MultipleStruct(multi_struct) = multiple_struct {
            multi_struct.0.push(struct_def);
        } else {
            panic!("Expected MultipleStruct variant");
        }
    }

    fn set_last_struct(&mut self, value: Option<(&'a str, &'a str)>) {
        self.last_struct = value;
    }

    fn get_flags_id(&self, name: &str) -> Option<&'a str> {
        self.named_bitflags.get(name).copied()
    }

    fn get_struct_id(&self, name: &str) -> Option<&'a str> {
        self.named_structs.get(name).copied()
    }

    fn insert_flags_variable(&mut self, name: &'a str, flags: PatternArg<'a>) {
        let id = self.generate_variable_id();
        self.insert_named_bitflag(name, id);
        self.add_flags_variable(id, flags);
        self.set_last_struct(None);
    }

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

fn identifier(input: &str) -> IResult<&str, &str> {
    alt((
        nom::character::complete::digit1,
        recognize(pair(
            alt((nom::character::complete::alpha1, tag("_"))),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
    ))(input)
}

impl Patterns<'_> {
    /// Get patterns from SCML content with a specific variable prefix.
    fn from_scml_with_var_prefix(
        variable_prefix: String,
        content: &str,
    ) -> Result<Self, ScmlParseError> {
        let stmt_iterator = StatementIterator::new(content);
        let mut patterns: HashMap<&str, Vec<Pattern>> = HashMap::new();
        let mut ctx = ParserCtxBuilder::new(variable_prefix);
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

        if !errors.is_empty() {
            return Err(ScmlParseError::ParseError(errors.join("\n")));
        }

        Ok(Self::new(patterns, ctx.build()))
    }

    /// Merges another Patterns instance into self.
    fn merge(&mut self, other: Self) {
        for (syscall_name, mut other_patterns) in other.patterns {
            self.patterns
                .entry(syscall_name)
                .or_default()
                .append(&mut other_patterns);
        }

        self.ctx.flags_variables.extend(other.ctx.flags_variables);
        self.ctx.struct_variables.extend(other.ctx.struct_variables);
        self.ctx
            .multiple_struct_variables
            .extend(other.ctx.multiple_struct_variables);
    }
}

impl<'a> Pattern<'a> {
    fn parse_param_list(
        ctx: &ParserCtxBuilder<'a>,
        input: &'a str,
    ) -> IResult<&'a str, Vec<PatternArg<'a>>> {
        separated_list0(delimited(multispace0, char(','), multispace0), |i| {
            Self::parse_param(ctx, i)
        })(input)
    }

    fn parse_param(ctx: &ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
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

    fn parse_expr(ctx: &ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        alt((
            |i| Self::parse_struct(ctx, i),
            |i| Self::parse_array(ctx, i),
            Self::parse_builtin_type,
            |i| Self::parse_flags(ctx, i),
            |i| Self::parse_struct_variable(ctx, i),
            |i| Self::parse_flags_variable(ctx, i),
        ))(input)
    }

    fn parse_struct(
        ctx: &ParserCtxBuilder<'a>,
        input: &'a str,
    ) -> IResult<&'a str, PatternArg<'a>> {
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

    fn parse_struct_field(
        ctx: &ParserCtxBuilder<'a>,
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

    fn parse_array(ctx: &ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
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

    fn parse_flags(ctx: &ParserCtxBuilder<'a>, input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, flags) = separated_list1(
            delimited(multispace0, char('|'), multispace0),
            alt((Self::parse_builtin_type, Self::parse_flag, |i| {
                Self::parse_flags_variable(ctx, i)
            })),
        )(input)?;

        Ok((input, PatternArg::Flags(PatternFlagSet::new(flags))))
    }

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

    fn parse_flags_variable(
        ctx: &ParserCtxBuilder<'a>,
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

    fn parse_struct_variable(
        ctx: &ParserCtxBuilder<'a>,
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

    fn parse_flag(input: &'a str) -> IResult<&'a str, PatternArg<'a>> {
        let (input, _) = multispace0(input)?;
        let (input, flag_name) = identifier(input)?;
        Ok((input, PatternArg::Flag(flag_name)))
    }

    fn parse_flags_definition(
        ctx: &ParserCtxBuilder<'a>,
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

    fn parse_struct_definition(
        ctx: &ParserCtxBuilder<'a>,
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

/// Iterator that yields complete statements from SCML content.
struct StatementIterator<'a> {
    lines: std::str::Lines<'a>,
    current_statement: String,
}

impl<'a> StatementIterator<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            lines: content.lines(),
            current_statement: String::new(),
        }
    }
}

impl Iterator for StatementIterator<'_> {
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
