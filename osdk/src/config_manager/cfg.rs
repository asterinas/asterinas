// SPDX-License-Identifier: MPL-2.0

//! A module for handling configurations.

use std::{
    collections::BTreeMap,
    fmt::{self, Display},
};

/// A configuration that looks like "cfg(k1=v1, k2=v2, ...)".
#[derive(Debug, Clone, Eq, Ord, PartialOrd, PartialEq, Serialize)]
pub struct Cfg(BTreeMap<String, String>);

#[derive(Debug)]
pub struct CfgParseError(String);

impl fmt::Display for CfgParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to parse cfg: {}", self.0)
    }
}

impl serde::ser::StdError for CfgParseError {}
impl serde::de::Error for CfgParseError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self(msg.to_string())
    }
}

impl CfgParseError {
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// This allows literal constructions like `Cfg::from([("arch", "foo"), ("schema", "bar")])`.
impl<K, V, const N: usize> From<[(K, V); N]> for Cfg
where
    K: Into<String>,
    V: Into<String>,
{
    fn from(array: [(K, V); N]) -> Self {
        let mut cfg = BTreeMap::new();
        for (k, v) in array.into_iter() {
            cfg.insert(k.into(), v.into());
        }
        Self(cfg)
    }
}

impl Cfg {
    pub fn empty() -> Self {
        Self(BTreeMap::new())
    }

    pub fn from_str(s: &str) -> Result<Self, CfgParseError> {
        let s = s.trim();

        // Match the leading "cfg(" and trailing ")"
        if !s.starts_with("cfg(") || !s.ends_with(')') {
            return Err(CfgParseError::new(s));
        }
        let s = &s[4..s.len() - 1];

        let mut cfg = BTreeMap::new();
        for kv in s.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let kv: Vec<_> = kv.split('=').collect();
            if kv.len() != 2 {
                return Err(CfgParseError::new(s));
            }
            cfg.insert(
                kv[0].trim().to_string(),
                kv[1].trim().trim_matches('\"').to_string(),
            );
        }
        Ok(Self(cfg))
    }

    pub fn map(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

impl Display for Cfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cfg(")?;
        for (i, (k, v)) in self.0.iter().enumerate() {
            write!(f, "{}=\"{}\"", k, v)?;
            if i != self.0.len() - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, ")")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_cfg_from_str() {
        let cfg = Cfg::from([("arch", "x86_64"), ("schema", "foo")]);
        let cfg1 = Cfg::from_str("cfg(arch =  \"x86_64\",     schema=\"foo\", )").unwrap();
        let cfg2 = Cfg::from_str("cfg(arch=\"x86_64\",schema=\"foo\")").unwrap();
        let cfg3 = Cfg::from_str("cfg( arch=\"x86_64\", schema=\"foo\" )").unwrap();
        assert_eq!(cfg, cfg1);
        assert_eq!(cfg, cfg2);
        assert_eq!(cfg, cfg3);
    }

    #[test]
    fn test_cfg_display() {
        let cfg = Cfg::from([("arch", "x86_64"), ("schema", "foo")]);
        let cfg_string = cfg.to_string();
        let cfg_back = Cfg::from_str(&cfg_string).unwrap();
        assert_eq!(cfg_string, "cfg(arch=\"x86_64\", schema=\"foo\")");
        assert_eq!(cfg, cfg_back);
    }

    #[test]
    fn test_bad_cfg_strings() {
        assert!(Cfg::from_str("fg(,,,,arch=\"x86_64 \", schema=\"foo\")").is_err());
        assert!(Cfg::from_str("cfg(arch=\"x86_64\", schema=\"foo\"").is_err());
        assert!(Cfg::from_str("cfgarch=x86_64,,, schema=\"foo\") ").is_err());
    }
}
