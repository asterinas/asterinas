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
    target::{
        DmTarget, error::ErrorTarget, linear::LinearTarget, verity::VerityTarget, zero::ZeroTarget,
    },
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
        "verity" => Box::new(parse_verity_target(&args)?),
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

fn parse_verity_target(args: &[&str]) -> Result<VerityTarget, DmErrorWithContext> {
    VerityTarget::from_table_args(args)
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

/// Parses a single mandatory table field, attaching `what` to any error.
pub(crate) fn parse_field<T: FromStr>(value: &str, what: &str) -> Result<T, DmErrorWithContext> {
    value
        .parse::<T>()
        .map_err(|_| DmError::InvalidTable.context(alloc::format!("{} is invalid", what)))
}

pub(crate) fn parse_hex_bytes(input: &str) -> Result<Vec<u8>, DmError> {
    if input == "-" {
        return Ok(Vec::new());
    }
    if !input.len().is_multiple_of(2) {
        return Err(DmError::InvalidArgument);
    }

    let mut out = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let hi = hex_value(bytes[offset]).ok_or(DmError::InvalidArgument)?;
        let lo = hex_value(bytes[offset + 1]).ok_or(DmError::InvalidArgument)?;
        out.push((hi << 4) | lo);
        offset += 2;
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::{DmCreateArg, parse_create_arg, parse_hex_bytes};
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

    #[ktest]
    fn parse_hex_bytes_round_trips_and_validates() {
        assert_eq!(
            parse_hex_bytes("00ff10").unwrap(),
            alloc::vec![0x00, 0xff, 0x10]
        );
        assert!(parse_hex_bytes("0").is_err());
        assert!(parse_hex_bytes("gg").is_err());
        assert!(parse_hex_bytes("-").unwrap().is_empty());
    }
}
