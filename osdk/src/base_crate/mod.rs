// SPDX-License-Identifier: MPL-2.0

//! The base crate is the OSDK generated crate that is ultimately built by cargo.
//! It will depend on the to-be-built kernel crate or the to-be-tested crate.

use std::{
    fs,
    io::{Read, Result},
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::util::get_cargo_metadata;

/// Compares two files byte-by-byte to check if they are identical.
/// Returns `Ok(true)` if files are identical, `Ok(false)` if they are different, or `Err` if any I/O operation fails.
fn are_files_identical(file1: &Path, file2: &Path) -> Result<bool> {
    // Check file size first
    let metadata1 = fs::metadata(file1)?;
    let metadata2 = fs::metadata(file2)?;

    if metadata1.len() != metadata2.len() {
        return Ok(false); // Different sizes, not identical
    }

    // Compare file contents byte-by-byte
    let mut file1 = fs::File::open(file1)?;
    let mut file2 = fs::File::open(file2)?;

    let mut buffer1 = [0u8; 4096];
    let mut buffer2 = [0u8; 4096];

    loop {
        let bytes_read1 = file1.read(&mut buffer1)?;
        let bytes_read2 = file2.read(&mut buffer2)?;

        if bytes_read1 != bytes_read2 || buffer1[..bytes_read1] != buffer2[..bytes_read1] {
            return Ok(false); // Files are different
        }

        if bytes_read1 == 0 {
            return Ok(true); // End of both files, identical
        }
    }
}

const GENERATED_BASE_FILES: &[&str] = &["Cargo.toml", "src/main.rs"];
const GENERATED_LINKER_SCRIPTS: &[(&str, &str)] = &[
    ("x86_64.ld", include_str!("x86_64.ld.template")),
    ("riscv64.ld", include_str!("riscv64.ld.template")),
    ("loongarch64.ld", include_str!("loongarch64.ld.template")),
];

fn generated_base_files_are_identical(existing: &Path, regenerated: &Path) -> bool {
    GENERATED_BASE_FILES
        .iter()
        .copied()
        .chain(
            GENERATED_LINKER_SCRIPTS
                .iter()
                .map(|(relative_path, _)| *relative_path),
        )
        .all(|relative_path| {
            are_files_identical(
                &existing.join(relative_path),
                &regenerated.join(relative_path),
            )
            .is_ok_and(|identical| identical)
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseCrateType {
    /// The base crate is for running the target kernel crate.
    Run,
    /// The base crate is for testing the target crate.
    Test,
    /// The base crate is for other actions using Cargo.
    #[expect(unused)]
    Other,
}

/// Create a new base crate that will be built by cargo.
///
/// The dependencies of the base crate will be the target crate. If
/// `link_unit_test_kernel` is set to true, the base crate will also depend on
/// the `ostd-test-kernel` crate.
///
/// It returns the path to the base crate.
pub fn new_base_crate(
    base_type: BaseCrateType,
    base_crate_path_stem: impl AsRef<Path>,
    dep_crate_name: &str,
    dep_crate_path: impl AsRef<Path>,
    link_unit_test_kernel: bool,
) -> PathBuf {
    let base_crate_path: PathBuf = PathBuf::from(
        (base_crate_path_stem.as_ref().as_os_str().to_string_lossy()
            + match base_type {
                BaseCrateType::Run => "-run-base",
                BaseCrateType::Test => "-test-base",
                BaseCrateType::Other => "-base",
            })
        .to_string(),
    );
    // Check if the existing crate base is reusable.
    if base_type == BaseCrateType::Run && base_crate_path.exists() {
        // Reuse the existing base crate if it is identical to the new one.
        let base_crate_tmp_path = base_crate_path.join("tmp");
        do_new_base_crate(
            &base_crate_tmp_path,
            dep_crate_name,
            &dep_crate_path,
            link_unit_test_kernel,
        );
        let can_reuse = generated_base_files_are_identical(&base_crate_path, &base_crate_tmp_path);
        std::fs::remove_dir_all(&base_crate_tmp_path).unwrap();
        if can_reuse {
            info!("Reusing existing base crate");
            return base_crate_path;
        }
    }
    do_new_base_crate(
        &base_crate_path,
        dep_crate_name,
        dep_crate_path,
        link_unit_test_kernel,
    );

    base_crate_path
}

fn do_new_base_crate(
    base_crate_path: impl AsRef<Path>,
    dep_crate_name: &str,
    dep_crate_path: impl AsRef<Path>,
    link_unit_test_kernel: bool,
) {
    let workspace_root = {
        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
    };

    if base_crate_path.as_ref().exists() {
        std::fs::remove_dir_all(&base_crate_path).unwrap();
    }

    let dep_meta = get_cargo_metadata(Some(dep_crate_path.as_ref()), None::<&[&str]>).unwrap();
    let (dep_crate_version, dep_crate_features) = {
        let dep_meta = dep_meta.as_object().unwrap();

        let dep_package = {
            let packages = dep_meta.get("packages").unwrap().as_array().unwrap();
            packages
                .iter()
                .find(|package| {
                    let package = package.as_object().unwrap();
                    let name = package.get("name").unwrap().as_str().unwrap();
                    name == dep_crate_name
                })
                .unwrap()
                .as_object()
                .unwrap()
        };
        let dep_version = dep_package.get("version").unwrap().as_str().unwrap();
        let dep_features = dep_package.get("features").unwrap().as_object().unwrap();
        (dep_version, dep_features)
    };

    // Create the directory
    fs::create_dir_all(&base_crate_path).unwrap();
    // Create the src directory
    fs::create_dir_all(base_crate_path.as_ref().join("src")).unwrap();

    // Write Cargo.toml
    let cargo_toml = include_str!("Cargo.toml.template");
    let cargo_toml = cargo_toml.replace("#NAME#", &(dep_crate_name.to_string() + "-osdk-bin"));
    let cargo_toml = cargo_toml.replace("#VERSION#", dep_crate_version);
    fs::write(base_crate_path.as_ref().join("Cargo.toml"), cargo_toml).unwrap();

    // Set the current directory to the target osdk directory
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();

    for (relative_path, contents) in GENERATED_LINKER_SCRIPTS {
        fs::write(base_crate_path.as_ref().join(relative_path), contents).unwrap();
    }

    // Overwrite the main.rs file
    let main_rs = include_str!("main.rs.template");
    // Replace all occurrence of `#TARGET_NAME#` with the `dep_crate_name`
    let main_rs = main_rs.replace("#TARGET_NAME#", &dep_crate_name.replace('-', "_"));
    fs::write("src/main.rs", main_rs).unwrap();

    // Add dependencies to the Cargo.toml
    add_manifest_dependency(dep_crate_name, dep_crate_path, link_unit_test_kernel);

    // Copy the manifest configurations from the target crate to the base crate
    copy_profile_configurations(workspace_root);

    // Generate the features by copying the features from the target crate
    let dep_crate_features = dep_crate_features.iter().map(|(feature, value)| {
        let array = value
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .map(toml::Value::String)
            .collect();
        (feature.clone(), toml::Value::Array(array))
    });
    add_feature_entries(dep_crate_name, dep_crate_features);

    // Get back to the original directory
    std::env::set_current_dir(original_dir).unwrap();
}

fn add_manifest_dependency(
    crate_name: &str,
    crate_path: impl AsRef<Path>,
    link_unit_test_kernel: bool,
) {
    let manifest_path = "Cargo.toml";

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    // Check if "dependencies" key exists, create it if it doesn't
    if !manifest.contains_key("dependencies") {
        manifest.insert(
            "dependencies".to_string(),
            toml::Value::Table(toml::Table::new()),
        );
    }

    let dependencies = manifest.get_mut("dependencies").unwrap();

    // We disable default features when depending on the target crate, and add
    // all the default features as the default features of the base crate, to
    // allow controls from the users.
    // See `add_feature_entries` for more details.
    let target_dep = toml::Table::from_str(&format!(
        "{} = {{ path = \"{}\", default-features = false }}",
        crate_name,
        crate_path.as_ref().display()
    ))
    .unwrap();
    dependencies.as_table_mut().unwrap().extend(target_dep);

    if link_unit_test_kernel {
        add_manifest_dependency_to(
            dependencies,
            "osdk-test-kernel",
            Path::new("deps").join("test-kernel"),
        );
    }

    add_manifest_dependency_to(
        dependencies,
        "osdk-frame-allocator",
        Path::new("deps").join("frame-allocator"),
    );

    add_manifest_dependency_to(
        dependencies,
        "osdk-heap-allocator",
        Path::new("deps").join("heap-allocator"),
    );

    add_manifest_dependency_to(dependencies, "ostd", Path::new("..").join("ostd"));

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}

fn add_manifest_dependency_to(manifest: &mut toml::Value, dep_name: &str, path: PathBuf) {
    let dep_str = match option_env!("OSDK_LOCAL_DEV") {
        Some("1") => {
            let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dep_crate_dir = crate_dir.join(path);
            format!(
                "{} = {{ path = \"{}\" }}",
                dep_name,
                dep_crate_dir.display()
            )
        }
        _ => format!(
            "{} = {{ version = \"{}\" }}",
            dep_name,
            env!("CARGO_PKG_VERSION"),
        ),
    };
    let dep_val = toml::Table::from_str(&dep_str).unwrap();
    manifest.as_table_mut().unwrap().extend(dep_val);
}

fn copy_profile_configurations(workspace_root: impl AsRef<Path>) {
    let target_manifest_path = workspace_root.as_ref().join("Cargo.toml");
    let manifest_path = "Cargo.toml";

    let target_manifest: toml::Table = {
        let content = fs::read_to_string(target_manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    // Copy the profile configurations
    let profile = target_manifest.get("profile");
    if let Some(profile) = profile {
        manifest.insert(
            "profile".to_string(),
            toml::Value::Table(profile.as_table().unwrap().clone()),
        );
    }

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}

fn add_feature_entries(
    dep_crate_name: &str,
    features: impl Iterator<Item = (String, toml::Value)>,
) {
    let manifest_path = "Cargo.toml";
    let mut manifest: toml::Table = {
        let content = fs::read_to_string(manifest_path).unwrap();
        toml::from_str(&content).unwrap()
    };

    let mut table = toml::Table::new();
    for (feature, value) in features {
        let value = if feature != "default" {
            vec![toml::Value::String(format!(
                "{}/{}",
                dep_crate_name, feature
            ))]
        } else {
            value.as_array().unwrap().clone()
        };
        table.insert(feature.clone(), toml::Value::Array(value));
    }

    manifest.insert("features".to_string(), toml::Value::Table(table));

    let content = toml::to_string(&manifest).unwrap();
    fs::write(manifest_path, content).unwrap();
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    const GENERATED_FILE_FIXTURES: &[(&str, &str)] = &[
        ("Cargo.toml", "[package]\nname = \"base\"\n"),
        ("src/main.rs", "fn main() {}\n"),
        ("x86_64.ld", "x86_64 linker script\n"),
        ("riscv64.ld", "riscv64 linker script\n"),
        ("loongarch64.ld", "loongarch64 linker script\n"),
    ];

    fn write_generated_base_crate(path: &Path) {
        fs::create_dir_all(path.join("src")).unwrap();
        for (relative_path, contents) in GENERATED_FILE_FIXTURES {
            fs::write(path.join(relative_path), contents).unwrap();
        }
    }

    #[test]
    fn changed_linker_script_prevents_base_crate_reuse() {
        // Regression test for https://github.com/asterinas/asterinas/issues/3649.
        let temp_dir = tempdir().unwrap();
        let existing = temp_dir.path().join("existing");
        let regenerated = temp_dir.path().join("regenerated");
        write_generated_base_crate(&existing);
        write_generated_base_crate(&regenerated);

        assert!(
            super::generated_base_files_are_identical(&existing, &regenerated),
            "identical generated base crates must remain reusable"
        );

        for linker_script in ["x86_64.ld", "riscv64.ld", "loongarch64.ld"] {
            fs::write(regenerated.join(linker_script), "changed linker script\n").unwrap();
            assert!(
                !super::generated_base_files_are_identical(&existing, &regenerated),
                "a change to {linker_script} must invalidate the generated base crate"
            );
            fs::copy(
                existing.join(linker_script),
                regenerated.join(linker_script),
            )
            .unwrap();
        }
    }

    #[test]
    fn missing_linker_script_prevents_base_crate_reuse() {
        let temp_dir = tempdir().unwrap();
        let existing = temp_dir.path().join("existing");
        let regenerated = temp_dir.path().join("regenerated");
        write_generated_base_crate(&existing);
        write_generated_base_crate(&regenerated);
        fs::remove_file(regenerated.join("riscv64.ld")).unwrap();

        assert!(
            !super::generated_base_files_are_identical(&existing, &regenerated),
            "a missing linker script must invalidate the generated base crate"
        );
    }
}
