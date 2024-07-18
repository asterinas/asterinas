// SPDX-License-Identifier: MPL-2.0

use std::{collections::HashMap, path::PathBuf, process::Command, str::FromStr};

use json::JsonValue;
use proc_macro2::{Group, TokenStream};
use quote::{ToTokens, TokenStreamExt};

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

    let comps_name = get_components_name(&metadata["packages"]);
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

fn is_component(package: &JsonValue) -> bool {
    for depend in package["dependencies"].members() {
        if depend["name"].as_str().unwrap() == "component" {
            return true;
        }
    }
    false
}

/// Get all the components name, this function will also check if the Components.toml contain all the components.
fn get_components_name(packages: &JsonValue) -> Vec<String> {
    let mut comps_name = Vec::new();
    for package in packages.members() {
        if is_component(package) {
            comps_name.push(package["name"].as_str().unwrap().to_string());
        }
    }
    comps_name
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
