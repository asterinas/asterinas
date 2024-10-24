// SPDX-License-Identifier: MPL-2.0

//! Filtering Utility
//!
//! This module provides the abstraction `FilterEntry` to assist loggers with filtering
//! functionality, as well as the method `get_filter_list` to help read user-specified
//! `FilterEntry`. Users can add entries in the `filter_list` file in the same directory,
//! following the described format:
//!
//! Module type:
//! module <module_path> <log_level>
//!
//! File type:
//! file <file_path> <log_level>
//!
//! Line type:
//! line <file_path> <line_number> <log_level>
//!
//! # Examples
//! // in filter_list
//! module mod_foo::block::device Info
//!
//! In this case, users may want to focus the `info!` output in `mod_foo::block::device` module,
//! so they put this line to `filter_list` to change the max log level of `mod_foo::block::device`
//! module to `Info`, thus they can use `Error` as global max log level yet make the `info!` operations
//! still enabled without output info messages in other module.
//!
//! // in filter_list
//! file /root/my_dir/foo/src/io.rs Off
//!
//! In this case, we assume that the user wants to perform global debugging, but there is too much irrelevant
//! debug information output in the `/root/my_dir/foo/src/io.rs` file. Therefore, they add this path to
//! the `filter_list` to block these outputs.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::str::FromStr;

use log::{error, LevelFilter};

#[derive(Debug)]
/// `FilterEntry` is used to help the log system to do a fine-grained filtering.
/// There are three types of `FilterEntry`:
/// - Module type, means one want to change the [max log level] of the entire module
///   to `log_level`.
/// - File type, means one want to change the [max log level] of a specified file to
///   `log_level`.
/// - Line type, means one want to change the [max log level] of a line in a specified
///   file to `log_level`.
///
/// [max log level]: The max log level within a scope ensures that any log operations
/// with a log level greater than this value within that scope are ineffective.
pub(crate) enum FilterEntry {
    Module {
        module: String,
        log_level: LevelFilter,
    },
    File {
        file: String,
        log_level: LevelFilter,
    },
    Line {
        file: String,
        line_number: u32,
        log_level: LevelFilter,
    },
}

impl FilterEntry {
    /// Creates a `FilterEntry` from an input line.
    pub(crate) fn from_line(line: &str) -> Option<FilterEntry> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first() {
            Some(&"module") => {
                if parts.len() == 3 {
                    Some(FilterEntry::Module {
                        module: parts[1].to_string(),
                        log_level: LevelFilter::from_str(parts[2]).ok()?,
                    })
                } else {
                    None
                }
            }
            Some(&"file") => {
                if parts.len() == 3 {
                    Some(FilterEntry::File {
                        file: parts[1].to_string(),
                        log_level: LevelFilter::from_str(parts[2]).ok()?,
                    })
                } else {
                    None
                }
            }
            Some(&"line") => {
                if parts.len() == 4 {
                    if let Ok(line_number) = parts[2].parse::<u32>() {
                        Some(FilterEntry::Line {
                            file: parts[1].to_string(),
                            line_number,
                            log_level: LevelFilter::from_str(parts[3]).ok()?,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn log_level(&self) -> LevelFilter {
        match self {
            FilterEntry::Module { log_level, .. } => *log_level,
            FilterEntry::File { log_level, .. } => *log_level,
            FilterEntry::Line { log_level, .. } => *log_level,
        }
    }
}

pub(crate) fn get_filter_list() -> (Vec<FilterEntry>, LevelFilter) {
    let file = include_str!("filter_list");
    let mut filter_list = Vec::new();
    let mut filter_max_level = LevelFilter::Off;
    for line in file.lines() {
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        if let Some(entry) = FilterEntry::from_line(line) {
            let log_level = entry.log_level();
            if entry.log_level() > filter_max_level {
                filter_max_level = log_level;
            }

            filter_list.push(entry);
        } else {
            error!("Invalid logger filter entry: {}\n", line);
        }
    }

    (filter_list, filter_max_level)
}
