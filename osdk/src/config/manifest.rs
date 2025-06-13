// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    process,
};

use clap::ValueEnum;
use serde::{de, Deserialize, Deserializer, Serialize};

use super::scheme::Scheme;

use crate::{error::Errno, error_msg, util::get_cargo_metadata};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsdkMeta {
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
    #[cfg_attr(not(test), expect(dead_code))]
    pub project_type: Option<ProjectType>,
    pub default_scheme: Scheme,
    pub map: HashMap<String, Scheme>,
}

impl TomlManifest {
    pub fn load() -> Self {
        let workspace_root = {
            let cargo_metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
            PathBuf::from(
                cargo_metadata
                    .get("workspace_root")
                    .unwrap()
                    .as_str()
                    .unwrap(),
            )
        };

        // Search for OSDK.toml in the current directory first.
        let current_manifest_path = PathBuf::from("OSDK.toml").canonicalize();
        let current_manifest = match &current_manifest_path {
            Ok(path) => deserialize_toml_manifest(path),
            Err(_) => {
                // If not found, search in the workspace root.
                if let Ok(workspace_manifest_path) = workspace_root.join("OSDK.toml").canonicalize()
                {
                    deserialize_toml_manifest(workspace_manifest_path)
                } else {
                    None
                }
            }
        };

        let Some(mut current_manifest) = current_manifest else {
            error_msg!("Cannot find `OSDK.toml` in the current directory or the workspace root");
            process::exit(Errno::GetMetadata as _);
        };

        // Running and testing configs should inherit from the global configs.
        current_manifest
            .default_scheme
            .run_and_test_inherit_from_global();

        // All the schemes should inherit from the default scheme.
        for scheme in current_manifest.map.values_mut() {
            scheme.run_and_test_inherit_from_global();
            scheme.inherit(&current_manifest.default_scheme);
        }

        current_manifest
    }

    /// Get the scheme given the scheme from the command line arguments.
    pub fn get_scheme(&self, scheme: Option<impl ToString>) -> &Scheme {
        if let Some(scheme) = scheme {
            log::info!("Using scheme `{}`", scheme.to_string());

            let Some(selected_scheme) = self.map.get(&scheme.to_string()) else {
                error_msg!("Scheme `{}` not found in `OSDK.toml`", scheme.to_string());
                process::exit(Errno::ParseMetadata as _);
            };

            selected_scheme
        } else {
            log::info!("Using default scheme");

            &self.default_scheme
        }
    }
}

fn deserialize_toml_manifest(path: impl AsRef<Path>) -> Option<TomlManifest> {
    if !path.as_ref().exists() || !path.as_ref().is_file() {
        return None;
    }
    // Read the file content
    let contents = fs::read_to_string(&path).unwrap_or_else(|err| {
        error_msg!(
            "Cannot read file {}, {}",
            path.as_ref().to_string_lossy(),
            err,
        );
        process::exit(Errno::GetMetadata as _);
    });
    // Parse the TOML content
    let mut manifest: TomlManifest = toml::from_str(&contents).unwrap_or_else(|err| {
        let span = err.span().unwrap();
        let wider_span =
            (span.start as isize - 20).max(0) as usize..(span.end + 20).min(contents.len());
        error_msg!(
            "Cannot parse TOML file, {}. {}:{:?}:\n {}",
            err.message(),
            path.as_ref().to_string_lossy(),
            span,
            &contents[wider_span],
        );
        process::exit(Errno::ParseMetadata as _);
    });

    // Preprocess the parsed manifest
    let cwd = path.as_ref().parent().unwrap();
    manifest.default_scheme.work_dir = Some(cwd.to_path_buf());
    for scheme in manifest.map.values_mut() {
        scheme.work_dir = Some(cwd.to_path_buf());
    }

    Some(manifest)
}

impl<'de> Deserialize<'de> for TomlManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            ProjectType,
            SupportedArchs,
            Boot,
            Grub,
            Qemu,
            Build,
            Run,
            Test,
            Scheme,
        }

        const EXPECTED: &[&str] = &[
            "project_type",
            "supported_archs",
            "boot",
            "grub",
            "qemu",
            "build",
            "run",
            "test",
            "scheme",
        ];

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl de::Visitor<'_> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str(&EXPECTED.join(", "))
                    }

                    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        match v {
                            "project_type" => Ok(Field::ProjectType),
                            "supported_archs" => Ok(Field::SupportedArchs),
                            "boot" => Ok(Field::Boot),
                            "grub" => Ok(Field::Grub),
                            "qemu" => Ok(Field::Qemu),
                            "build" => Ok(Field::Build),
                            "run" => Ok(Field::Run),
                            "test" => Ok(Field::Test),
                            "scheme" => Ok(Field::Scheme),
                            _ => Err(de::Error::unknown_field(v, EXPECTED)),
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
                formatter.write_str("Scheme")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut project_type = None;
                let mut default_scheme = Scheme::empty();
                let mut scheme_map = HashMap::<String, Scheme>::new();

                macro_rules! match_and_add_option {
                    ($field:ident) => {{
                        let value = map.next_value()?;
                        if default_scheme.$field.is_some() {
                            error_msg!("Duplicated field `{}`", stringify!($field));
                            process::exit(Errno::ParseMetadata as _);
                        }
                        default_scheme.$field = Some(value);
                    }};
                }
                macro_rules! match_and_add_vec {
                    ($field:ident) => {{
                        let value = map.next_value()?;
                        if !default_scheme.$field.is_empty() {
                            error_msg!("Duplicated field `{}`", stringify!($field));
                            process::exit(Errno::ParseMetadata as _);
                        }
                        default_scheme.$field = value;
                    }};
                }

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::ProjectType => {
                            let value: ProjectType = map.next_value()?;
                            project_type = Some(value);
                        }
                        Field::SupportedArchs => match_and_add_vec!(supported_archs),
                        Field::Boot => match_and_add_option!(boot),
                        Field::Grub => match_and_add_option!(grub),
                        Field::Qemu => match_and_add_option!(qemu),
                        Field::Build => match_and_add_option!(build),
                        Field::Run => match_and_add_option!(run),
                        Field::Test => match_and_add_option!(test),
                        Field::Scheme => {
                            let scheme: HashMap<String, Scheme> = map.next_value()?;
                            scheme_map = scheme;
                        }
                    }
                }

                Ok(TomlManifest {
                    project_type,
                    default_scheme,
                    map: scheme_map,
                })
            }
        }

        deserializer.deserialize_struct("TomlManifest", EXPECTED, TomlManifestVisitor)
    }
}
