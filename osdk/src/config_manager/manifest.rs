// SPDX-License-Identifier: MPL-2.0

use std::{collections::BTreeMap, fmt, path::Path, process};

use clap::ValueEnum;
use serde::{de, Deserialize, Deserializer, Serialize};

use super::{action::ActionSettings, cfg::Cfg};

use crate::{config_manager::Arch, error::Errno, error_msg};

/// The settings for the actions summarized from the command line arguments
/// and the configuration file `OSDK.toml`.
#[derive(Debug, Clone)]
pub struct OsdkManifest {
    pub project: Project,
    pub run: Option<ActionSettings>,
    pub test: Option<ActionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(rename(serialize = "type", deserialize = "type"))]
    pub type_: ProjectType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectType {
    Kernel,
    #[value(alias("lib"))]
    Library,
    Module,
}

/// The osdk manifest from configuration file `OSDK.toml`.
#[derive(Debug, Clone)]
pub struct TomlManifest {
    pub project: Project,
    cfg_map: BTreeMap<Cfg, CfgArgs>,
}

impl TomlManifest {
    /// Get the action manifest given the architecture and the schema from the command line arguments.
    ///
    /// If any entry in the `OSDK.toml` manifest doesn't specify an architecture, we regard it matching
    /// all the architectures.
    pub fn get_osdk_manifest(
        &self,
        path_of_self: impl AsRef<Path>,
        arch: Arch,
        schema: Option<String>,
    ) -> OsdkManifest {
        let filtered_by_arch = self.cfg_map.iter().filter(|(cfg, _)| {
            if let Some(got) = cfg.map().get("arch") {
                got == &arch.to_string()
            } else {
                true
            }
        });

        let filtered_by_schema = if let Some(schema) = schema {
            filtered_by_arch
                .filter(|(cfg, _)| {
                    if let Some(got) = cfg.map().get("schema") {
                        got == &schema
                    } else {
                        false
                    }
                })
                .collect::<Vec<_>>()
        } else {
            filtered_by_arch
                .filter(|(cfg, _)| cfg == &&Cfg::empty())
                .collect::<Vec<_>>()
        };

        let filtered = filtered_by_schema;
        if filtered.len() > 1 {
            error_msg!("Multiple entries in OSDK.toml match the given architecture and schema");
            process::exit(Errno::ParseMetadata as _);
        }
        if filtered.is_empty() {
            error_msg!("No entry in OSDK.toml matches the given architecture and schema");
            process::exit(Errno::ParseMetadata as _);
        }
        let final_cfg_args = filtered.first().unwrap().1;
        let mut run = final_cfg_args.run.clone();
        if let Some(run_inner) = &mut run {
            run_inner.canonicalize_paths(&path_of_self);
        }
        let mut test = final_cfg_args.test.clone();
        if let Some(test_inner) = &mut test {
            test_inner.canonicalize_paths(&path_of_self);
        }
        OsdkManifest {
            project: self.project.clone(),
            run,
            test,
        }
    }
}

/// A inner adapter for `TomlManifest` to allow the `cfg` field to be optional.
/// The fields should be identical to `TomlManifest` except the `cfg` field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CfgArgs {
    pub run: Option<ActionSettings>,
    pub test: Option<ActionSettings>,
}

impl CfgArgs {
    pub fn try_accept(&mut self, another: CfgArgs) {
        if another.run.is_some() {
            if self.run.is_some() {
                error_msg!("Duplicate `run` field in OSDK.toml");
                process::exit(Errno::ParseMetadata as _);
            }
            self.run = another.run;
        }
        if another.test.is_some() {
            if self.test.is_some() {
                error_msg!("Duplicate `test` field in OSDK.toml");
                process::exit(Errno::ParseMetadata as _);
            }
            self.test = another.test;
        }
    }
}

impl<'de> Deserialize<'de> for TomlManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Project,
            Run,
            Test,
            Cfg(Cfg),
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl<'de> de::Visitor<'de> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("`project`, `run`, `test` or cfg")
                    }

                    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        match v {
                            "project" => Ok(Field::Project),
                            "run" => Ok(Field::Run),
                            "test" => Ok(Field::Test),
                            v => Ok(Field::Cfg(Cfg::from_str(v).unwrap_or_else(|e| {
                                error_msg!("Error parsing cfg: {}", e);
                                process::exit(Errno::ParseMetadata as _);
                            }))),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct TomlManifestVisitor;

        impl<'de> de::Visitor<'de> for TomlManifestVisitor {
            type Value = TomlManifest;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct TomlManifest")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut project: Option<Project> = None;
                let default_cfg = Cfg::empty();
                let mut cfg_map = BTreeMap::<Cfg, CfgArgs>::new();

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Project => {
                            let value = map.next_value()?;
                            project = Some(value);
                        }
                        Field::Run => {
                            let value: ActionSettings = map.next_value()?;
                            cfg_map
                                .entry(default_cfg.clone())
                                .and_modify(|v| {
                                    v.try_accept(CfgArgs {
                                        run: Some(value.clone()),
                                        test: None,
                                    })
                                })
                                .or_insert(CfgArgs {
                                    run: Some(value.clone()),
                                    test: None,
                                });
                        }
                        Field::Test => {
                            let value: ActionSettings = map.next_value()?;
                            cfg_map
                                .entry(default_cfg.clone())
                                .and_modify(|v| {
                                    v.try_accept(CfgArgs {
                                        run: None,
                                        test: Some(value.clone()),
                                    })
                                })
                                .or_insert(CfgArgs {
                                    run: None,
                                    test: Some(value.clone()),
                                });
                        }
                        Field::Cfg(cfg) => {
                            let value: CfgArgs = map.next_value()?;
                            cfg_map
                                .entry(cfg)
                                .and_modify(|v| v.try_accept(value.clone()))
                                .or_insert(value.clone());
                        }
                    }
                }

                Ok(TomlManifest {
                    project: project.unwrap_or_else(|| {
                        error_msg!("`project` field is required in OSDK.toml");
                        process::exit(Errno::ParseMetadata as _);
                    }),
                    cfg_map,
                })
            }
        }

        deserializer.deserialize_struct(
            "TomlManifest",
            &["run", "test", "cfg"],
            TomlManifestVisitor,
        )
    }
}
