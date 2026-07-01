// SPDX-License-Identifier: MPL-2.0

use alloc::{
    boxed::Box,
    string::{String, ToString},
    sync::Arc,
    vec::Vec,
};
use core::str::FromStr;

use aster_block::BlockDevice;
use aster_cmdline::parse::ParamError;
use device_id::DeviceId;

use crate::{
    DmError, DmErrorWithContext,
    registry::normalize_name,
    table::{DmTable, DmTableSegment},
    target::{DmTarget, error::ErrorTarget, linear::LinearTarget, zero::ZeroTarget},
};

/// One `dm-mod.create=` value from the kernel command line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DmCreateArg(String);

impl DmCreateArg {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for DmCreateArg {
    type Err = ParamError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // A dm table value contains spaces, so on the kernel command line it is
        // written as `dm_mod.create="<name>: <start> <len> <target> ..."`. The
        // command-line tokenizer keeps the wrapping double quotes in the value,
        // so strip one matching pair here, mirroring how Linux unquotes kernel
        // parameter values in `lib/cmdline.c`.
        let unquoted = strip_matching_quotes(s.trim());
        if unquoted.is_empty() {
            Err(ParamError::InvalidValue)
        } else {
            Ok(Self(unquoted.to_string()))
        }
    }
}

fn strip_matching_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(value)
}

#[derive(Debug)]
pub struct ParsedDmCreate {
    pub name: String,
    pub table: DmTable,
}

pub(crate) fn parse_create_arg(
    arg: &str,
    fallback_index: usize,
) -> Result<ParsedDmCreate, DmErrorWithContext> {
    let (raw_name, table_text) = split_name_and_table(arg)
        .ok_or_else(|| DmError::InvalidTable.context("dm create argument must include a table"))?;
    let name = normalize_name(raw_name.trim(), fallback_index);
    let mut segments = Vec::new();
    for line in table_text.split(';') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        segments.push(parse_segment(line)?);
    }

    let table = DmTable::new(segments)
        .map_err(|err| err.context("dm table segments are empty, overlapping, or invalid"))?;
    Ok(ParsedDmCreate { name, table })
}

fn split_name_and_table(arg: &str) -> Option<(&str, &str)> {
    if let Some((name, table)) = arg.split_once(':') {
        return Some((name, table));
    }
    if let Some((name, table)) = arg.split_once(',') {
        return Some((name, table));
    }

    // Linux dm-init accepts a richer comma-separated grammar. A whitespace-only
    // value is interpreted as a single anonymous table.
    Some(("-", arg))
}

fn parse_segment(line: &str) -> Result<DmTableSegment, DmErrorWithContext> {
    let mut fields = line.split_whitespace();
    let start_sector = parse_u64(fields.next(), "segment start sector")?;
    let len_sectors = parse_u64(fields.next(), "segment length")?;
    let target_name = fields
        .next()
        .ok_or_else(|| DmError::InvalidTable.context("segment target type is missing"))?;
    let args: Vec<&str> = fields.collect();

    let target: Box<dyn DmTarget> = match target_name {
        "linear" => Box::new(parse_linear_target(&args)?),
        "zero" => {
            if !args.is_empty() {
                return Err(DmError::InvalidTable.context("zero target takes no arguments"));
            }
            Box::<ZeroTarget>::default()
        }
        "error" => {
            if !args.is_empty() {
                return Err(DmError::InvalidTable.context("error target takes no arguments"));
            }
            Box::<ErrorTarget>::default()
        }
        _ => return Err(DmError::UnsupportedTarget.context("unsupported dm target")),
    };
    if let Some(size_sectors) = target.size_sectors()
        && size_sectors != len_sectors
    {
        return Err(DmError::InvalidTable.context("target size does not match segment length"));
    }

    Ok(DmTableSegment {
        start_sector,
        len_sectors,
        target,
    })
}

fn parse_linear_target(args: &[&str]) -> Result<LinearTarget, DmErrorWithContext> {
    if args.len() != 2 {
        return Err(DmError::InvalidTable.context("linear target expects: <device> <start>"));
    }
    let device = lookup_block_device(args[0])?;
    let start_sector = parse_u64(args.get(1).copied(), "linear target start sector")?;
    Ok(LinearTarget::new(device, start_sector))
}

pub(crate) fn lookup_block_device(
    name_or_id: &str,
) -> Result<Arc<dyn BlockDevice>, DmErrorWithContext> {
    if let Some(raw) = name_or_id.strip_prefix("dev:") {
        let id = parse_u64(Some(raw), "encoded device id")?;
        let device_id = DeviceId::from_encoded_u64(id)
            .ok_or_else(|| DmError::InvalidArgument.context("encoded device id is invalid"))?;
        return aster_block::lookup(device_id)
            .ok_or_else(|| DmError::DeviceNotFound.context("block device id is not registered"));
    }

    aster_block::collect_all()
        .into_iter()
        .find(|device| device.name() == name_or_id)
        .ok_or_else(|| DmError::DeviceNotFound.context("block device name is not registered"))
}

fn parse_u64(value: Option<&str>, what: &str) -> Result<u64, DmErrorWithContext> {
    value
        .ok_or_else(|| DmError::InvalidTable.context(alloc::format!("{} is missing", what)))?
        .parse::<u64>()
        .map_err(|_| DmError::InvalidTable.context(alloc::format!("{} is invalid", what)))
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{DmCreateArg, parse_create_arg};
    use crate::DmError;

    #[ktest]
    fn dm_create_arg_strips_surrounding_quotes() {
        let arg = "\"dm-zero-test: 0 8 zero\"".parse::<DmCreateArg>().unwrap();
        assert_eq!(arg.as_str(), "dm-zero-test: 0 8 zero");
    }

    #[ktest]
    fn dm_create_arg_rejects_empty_value() {
        assert!("".parse::<DmCreateArg>().is_err());
        assert!("\"\"".parse::<DmCreateArg>().is_err());
    }

    #[ktest]
    fn parse_create_arg_accepts_single_segment() {
        let parsed = parse_create_arg("demo: 0 8 zero", 0).unwrap();
        assert_eq!(parsed.name, "demo");
        assert_eq!(parsed.table.total_sectors(), 8);
        let segments = parsed.table.segments();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].target.type_name(), "zero");
    }

    #[ktest]
    fn parse_create_arg_rejects_multiple_segments() {
        let result = parse_create_arg("demo: 0 8 zero; 8 8 error", 0);
        assert!(result.is_err());
    }

    #[ktest]
    fn parse_create_arg_rejects_overlapping_segments() {
        let result = parse_create_arg("demo: 0 8 zero; 4 8 error", 0);
        assert!(result.is_err());
    }

    #[ktest]
    fn parse_create_arg_rejects_unknown_target() {
        let result = parse_create_arg("demo: 0 8 bogus", 0);
        assert_eq!(result.unwrap_err().kind, DmError::UnsupportedTarget);
    }
}
