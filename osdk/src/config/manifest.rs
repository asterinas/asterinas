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

use crate::{config::scheme::QemuScheme, error::Errno, error_msg, util::get_cargo_metadata};

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
    #[allow(dead_code)]
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
        // All the custom schemes should inherit settings from the default scheme, this is a helper.
        fn finalize(current_manifest: Option<TomlManifest>) -> TomlManifest {
            let Some(mut current_manifest) = current_manifest else {
                error_msg!(
                    "Cannot find `OSDK.toml` in the current directory or the workspace root"
                );
                process::exit(Errno::GetMetadata as _);
            };
            for scheme in current_manifest.map.values_mut() {
                scheme.inherit(&current_manifest.default_scheme);
            }
            current_manifest
        }

        // Search for OSDK.toml in the current directory first.
        let current_manifest_path = PathBuf::from("OSDK.toml").canonicalize().ok();
        let mut current_manifest = match &current_manifest_path {
            Some(path) => deserialize_toml_manifest(path),
            None => None,
        };
        // Then search in the workspace root.
        let workspace_manifest_path = workspace_root.join("OSDK.toml").canonicalize().ok();
        // The case that the current directory is also the workspace root.
        if let Some(current) = &current_manifest_path {
            if let Some(workspace) = &workspace_manifest_path {
                if current == workspace {
                    return finalize(current_manifest);
                }
            }
        }
        let workspace_manifest = match workspace_manifest_path {
            Some(path) => deserialize_toml_manifest(path),
            None => None,
        };
        // The current manifest should inherit settings from the workspace manifest.
        if let Some(workspace_manifest) = workspace_manifest {
            if current_manifest.is_none() {
                current_manifest = Some(workspace_manifest);
            } else {
                // Inherit one scheme at a time.
                let current_manifest = current_manifest.as_mut().unwrap();
                current_manifest
                    .default_scheme
                    .inherit(&workspace_manifest.default_scheme);
                for (scheme_string, scheme) in workspace_manifest.map {
                    let current_scheme = current_manifest
                        .map
                        .entry(scheme_string)
                        .or_insert_with(Scheme::empty);
                    current_scheme.inherit(&scheme);
                }
            }
        }
        finalize(current_manifest)
    }

    /// Get the scheme given the scheme from the command line arguments.
    pub fn get_scheme(&self, scheme: Option<impl ToString>) -> &Scheme {
        if let Some(scheme) = scheme {
            let selected_scheme = self.map.get(&scheme.to_string());
            if selected_scheme.is_none() {
                error_msg!("Scheme `{}` not found in `OSDK.toml`", scheme.to_string());
                process::exit(Errno::ParseMetadata as _);
            }
            selected_scheme.unwrap()
        } else {
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
    // Canonicalize all the path fields
    let canonicalize = |target: &mut PathBuf| {
        let last_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(cwd).unwrap();
        *target = target.canonicalize().unwrap_or_else(|err| {
            error_msg!(
                "Cannot canonicalize path `{}`: {}",
                target.to_string_lossy(),
                err,
            );
            std::env::set_current_dir(&last_cwd).unwrap();
            process::exit(Errno::GetMetadata as _);
        });
        std::env::set_current_dir(last_cwd).unwrap();
    };
    let canonicalize_scheme = |scheme: &mut Scheme| {
        macro_rules! canonicalize_paths_in_scheme {
            ($scheme:expr) => {
                if let Some(ref mut boot) = $scheme.boot {
                    if let Some(ref mut initramfs) = boot.initramfs {
                        canonicalize(initramfs);
                    }
                }
                if let Some(ref mut qemu) = $scheme.qemu {
                    if let Some(ref mut qemu_path) = qemu.path {
                        canonicalize(qemu_path);
                    }
                }
                if let Some(ref mut grub) = $scheme.grub {
                    if let Some(ref mut grub_mkrescue_path) = grub.grub_mkrescue {
                        canonicalize(grub_mkrescue_path);
                    }
                }
            };
        }
        canonicalize_paths_in_scheme!(scheme);
        if let Some(ref mut run) = scheme.run {
            canonicalize_paths_in_scheme!(run);
        }
        if let Some(ref mut test) = scheme.test {
            canonicalize_paths_in_scheme!(test);
        }
    };
    canonicalize_scheme(&mut manifest.default_scheme);
    for scheme in manifest.map.values_mut() {
        canonicalize_scheme(scheme);
    }
    // Do evaluations on the need to be evaluated string field, namely,
    // QEMU arguments.
    use super::eval::eval;
    let eval_scheme = |scheme: &mut Scheme| {
        let eval_qemu = |qemu: &mut Option<QemuScheme>| {
            if let Some(ref mut qemu) = qemu {
                if let Some(ref mut args) = qemu.args {
                    *args = match eval(cwd, args) {
                        Ok(v) => v,
                        Err(e) => {
                            error_msg!("Failed to evaluate qemu args: {:#?}", e);
                            process::exit(Errno::ParseMetadata as _);
                        }
                    }
                }
            }
        };
        eval_qemu(&mut scheme.qemu);
        if let Some(ref mut run) = scheme.run {
            eval_qemu(&mut run.qemu);
        }
        if let Some(ref mut test) = scheme.test {
            eval_qemu(&mut test.qemu);
        }
    };
    eval_scheme(&mut manifest.default_scheme);
    for scheme in manifest.map.values_mut() {
        eval_scheme(scheme);
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

                impl<'de> de::Visitor<'de> for FieldVisitor {
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
