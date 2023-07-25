//! The module to parse kernel commandline arguments.
//! 

use alloc::string::String;

/// The struct to store parsed kernel commandline arguments.
pub struct KCmdlineArg {
    initproc: Option<String>,
}

// Define get APIs.
impl KCmdlineArg {
    pub fn get_initproc(&self) -> Option<&str> {
        self.initproc.as_deref()
    }
}

// Define the way to parse a string to `KCmdlineArg`.
impl From<&str> for KCmdlineArg {
    fn from(cmdline: &str) -> Self {
        KCmdlineArg { initproc: None }
    }
}
