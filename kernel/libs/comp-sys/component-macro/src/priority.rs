// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap, fs::File, io::Read, ops::Add, path::PathBuf, process::Command,
    str::FromStr,
};

use json::JsonValue;
use proc_macro2::{Group, TokenStream};
use quote::{ToTokens, TokenStreamExt};

use crate::COMPONENT_FILE_NAME;

#[derive(Debug)]
pub struct ComponentInfo {
    name: String,
    /// The absolute path to the component
    path: String,
    priority: u16,
}

impl ToTokens for ComponentInfo {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let token = TokenStream::from_str(
            format!("\"{}\",\"{}\",{}", self.name, self.path, self.priority).as_str(),
        )
        .unwrap();
        tokens.append(Group::new(proc_macro2::Delimiter::Parenthesis, token));
    }
}

/// Automatic generate all the component information
pub fn component_generate() -> Vec<ComponentInfo> {
    // extract components information
    let mut metadata = metadata();

    let mut component_packages = vec![];
    let workspace_root = metadata["workspace_root"].as_str().unwrap();
    let workspace_root = String::from_str(workspace_root).unwrap().replace('\\', "/");

    let comps_name = get_components_name(&workspace_root, &metadata["packages"]);
    for package in metadata["packages"].members_mut() {
        let name = package["name"].as_str().unwrap();
        if comps_name.contains(&name.to_string()) {
            // remove useless depend
            let mut depends = JsonValue::Array(Vec::new());
            loop {
                let depend = package["dependencies"].pop();
                if depend == JsonValue::Null {
                    break;
                }
                if comps_name.contains(&depend["name"].as_str().unwrap().to_string()) {
                    depends.push(depend).unwrap();
                }
            }
            package["dependencies"] = depends;
            component_packages.push(package);
        }
    }

    // calculate priority
    let mut mapping: HashMap<String, u16> = HashMap::new();
    let mut component_packages_map: HashMap<String, &mut JsonValue> = HashMap::new();
    for i in component_packages.iter_mut() {
        component_packages_map.insert(i["name"].as_str().unwrap().to_string(), i);
    }

    for (name, package) in component_packages_map.iter() {
        if mapping.contains_key(package["name"].as_str().unwrap()) {
            continue;
        }
        calculate_priority(&mut mapping, &component_packages_map, name.clone());
    }
    drop(component_packages_map);

    // priority calculation complete
    let mut components_info = Vec::new();
    for package in component_packages {
        let path = {
            // Parse the package ID <https://doc.rust-lang.org/cargo/reference/pkgid-spec.html>
            // and extract the path. Let's take `path+file:///path/to/comps/pci#aster-pci@0.1.0`
            // as an example package ID.
            let id = package["id"].as_str().unwrap();
            // Remove the prefix `path+file://`.
            assert!(id.starts_with("path+file://"));
            let id = id.trim_start_matches("path+file://");
            // Remove the fragment part `#aster-pci@0.1.0`. Note that the package name part
            // may be missing if the directory name is the same as the package name.
            id.split(['#', '@']).next().unwrap()
        };
        let component_info = {
            let package_name = package["name"].as_str().unwrap().to_string();
            ComponentInfo {
                name: package_name.clone(),
                path: PathBuf::from(&workspace_root)
                    .join(path)
                    .to_str()
                    .unwrap()
                    .to_string(),
                priority: *mapping.get(&package_name).unwrap(),
            }
        };
        components_info.push(component_info)
    }

    components_info
}

/// Get the path to the Components.toml file
pub fn get_component_toml_path() -> TokenStream {
    let metadata = metadata();
    let workspace_root = metadata["workspace_root"].as_str().unwrap();
    let mut workspace_root = String::from_str(workspace_root)
        .unwrap()
        .replace('\\', "/")
        .add("/")
        .add(COMPONENT_FILE_NAME)
        .add("\"");
    workspace_root.insert(0, '\"');
    TokenStream::from_str(workspace_root.as_str()).unwrap()
}

fn is_component(package: &JsonValue) -> bool {
    for depend in package["dependencies"].members() {
        if depend["name"].as_str().unwrap() == "component" {
            return true;
        }
    }
    false
}

/// Get all the components name, this function will also check if the Components.toml contain all the components.
fn get_components_name(workspace_root: &str, packages: &JsonValue) -> Vec<String> {
    let file_components_name = read_component_file(workspace_root);
    let mut comps_name = Vec::new();
    for package in packages.members() {
        if is_component(package) {
            if !file_components_name.contains(&package["name"].as_str().unwrap().to_string()) {
                // if the package is in the workspace_root
                if package["id"].as_str().unwrap().contains(workspace_root) {
                    panic!(
                        "Package {} in the workspace that not written in the {} file",
                        package["name"].as_str().unwrap(),
                        COMPONENT_FILE_NAME
                    );
                }
            }
            comps_name.push(package["name"].as_str().unwrap().to_string());
        }
    }
    comps_name
}

/// read component file, return all the components name
fn read_component_file(workspace_root: &str) -> Vec<String> {
    let component_toml: toml::Value = {
        let mut component_file_path = workspace_root.to_owned();
        component_file_path.push('/');
        component_file_path.push_str(COMPONENT_FILE_NAME);
        let mut file = File::open(component_file_path)
            .expect("Components.toml file not found, please check if the file exists");
        let mut str_val = String::new();
        file.read_to_string(&mut str_val).unwrap();
        toml::from_str(&str_val).unwrap()
    };
    for (name, value) in component_toml.as_table().unwrap() {
        if name.as_str() == "components" {
            return value
                .as_table()
                .unwrap()
                .values()
                .map(|value| {
                    value
                        .as_table()
                        .unwrap()
                        .values()
                        .map(|str_val| str_val.as_str().unwrap().to_string())
                        .collect()
                })
                .collect();
        }
    }
    panic!("Components.toml file not valid")
}

/// calculate the priority of one node
fn calculate_priority(
    prioritys: &mut HashMap<String, u16>,
    package_mapping: &HashMap<String, &mut JsonValue>,
    node_name: String,
) -> u16 {
    if prioritys.contains_key(&node_name) {
        return *prioritys.get(&node_name).unwrap();
    }

    let package = &package_mapping[&node_name];
    let mut lowest_priority: u16 = 0;
    for depends in package["dependencies"].members() {
        lowest_priority = lowest_priority.max(
            calculate_priority(
                prioritys,
                package_mapping,
                depends["name"].as_str().unwrap().to_string(),
            ) + 1,
        );
    }

    prioritys.insert(node_name.to_string(), lowest_priority);
    lowest_priority
}

fn metadata() -> json::JsonValue {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.arg("metadata");
    cmd.arg("--format-version").arg("1");
    let output = cmd.output().unwrap();

    if !output.status.success() {
        panic!("cannot get metadata");
    }

    let output = String::from_utf8(output.stdout).unwrap();
    json::parse(&output).unwrap()
}
