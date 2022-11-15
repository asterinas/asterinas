use super::file::{File, FileDescripter};

pub const FD_STDIN: FileDescripter = 0;
pub const FD_STDOUT: FileDescripter = 1;
pub const FD_STDERR: FileDescripter = 2;

#[derive(Debug)]
pub struct Stdin;

#[derive(Debug)]
pub struct Stdout;

#[derive(Debug)]
pub struct Stderr;

impl File for Stdin {}
impl File for Stdout {}
impl File for Stderr {}
