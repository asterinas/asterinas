// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

use std::collections::{BTreeMap, HashSet};
use std::{env, fs, io, path::PathBuf};

use once_cell::sync::OnceCell;
use toml::Value;

pub static CONFIG: OnceCell<Config> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct Config {
    components: BTreeMap<String, ComponentName>,
    whitelists: BTreeMap<Ident, WhiteList>,
}

impl Config {
    pub fn parse_toml(config_toml: Value) -> Self {
        let components_value = config_toml
            .get("components")
            .expect("The `components` key does not exist");
        let components = parse_components(components_value);
        let whitelist_value = config_toml
            .get("whitelist")
            .expect("The `whitelist` key does not exist");
        let whitelists = parse_whitelists(whitelist_value);
        Config {
            components,
            whitelists,
        }
    }

    pub fn ident_full_path(&self, ident: &Ident) -> Path {
        let component_ident = ident.iter().nth(0).unwrap();
        let component_path = self
            .components
            .get(component_ident)
            .expect("Undefined component ident")
            .clone();
        let component_libname = component_path.iter().last().unwrap();
        let mut ident_path = ident.clone();
        ident_path.remove_segment(0);
        ident_path.insert_segment(0, component_libname.clone());
        ident_path
    }

    pub fn component_path(&self, component_ident: &str) -> ComponentName {
        self.components
            .get(component_ident)
            .expect("Undefined component name")
            .clone()
    }

    pub fn allow_access(&self, crate_name: &str, def_path: &str) -> bool {
        let def_path = Path::from_qualified_str(def_path);
        for (ident, white_list) in &self.whitelists {
            let ident_full_path = self.ident_full_path(ident);
            if def_path == ident_full_path {
                for (component_ident, allowed) in white_list.iter() {
                    let component_lib_name = self.component_path(component_ident).filename();
                    if crate_name == &component_lib_name {
                        return *allowed;
                    }
                }
            }
        }
        false
    }

    /// ensure the config to be valid. We will check three things.
    /// 1. The component ident and library name(The last segment of component path) cannot be duplicate.
    /// 2. The controlled type in whilelist should be in one of defined components.
    /// 3. The components in whilelist should be defined.
    pub fn check_config(&self) {
        let mut component_idents = HashSet::new();
        let mut lib_names = HashSet::new();

        // check 1
        for (ident, component_path) in &self.components {
            if component_idents.contains(ident) {
                panic!("duplicate component ident");
            }
            component_idents.insert(ident.to_string());
            let lib_name = component_path.filename();
            if lib_names.contains(&lib_name) {
                panic!("duplicate library names");
            }
            lib_names.insert(lib_name);
        }

        for (type_, whilelist) in &self.whitelists {
            // check 2
            let component_ident = type_.iter().nth(0).unwrap();
            if !component_idents.contains(component_ident) {
                panic!("The controlled type is not in any component.");
            }
            // check 3
            for (component_name, _) in whilelist.iter() {
                if !component_idents.contains(component_name) {
                    panic!("The component in whitelist is not defined");
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct WhiteList {
    components: BTreeMap<String, bool>,
}

impl WhiteList {
    pub fn new() -> Self {
        WhiteList {
            components: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, component_name: &str, allowed: bool) {
        assert!(!self.components.contains_key(component_name));
        self.components.insert(component_name.to_string(), allowed);
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, String, bool> {
        self.components.iter()
    }
}

// rust crate name does not allow '-', so when we store ,all '-' will be replaced with '_'
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Path {
    segments: Vec<String>,
}

impl Path {
    pub fn from_str(path: &str) -> Self {
        let segments = path
            .split("/")
            .filter(|segment| segment.len() > 0)
            .map(|segment| segment.to_string().replace("-", "_"))
            .collect();
        Self { segments }
    }

    pub fn from_qualified_str(qualified_path: &str) -> Self {
        let segments = qualified_path
            .split("::")
            .filter(|segment| segment.len() > 0)
            .map(|segment| segment.to_string().replace("-", "_"))
            .collect();
        Self { segments }
    }

    pub fn from_segments(segments: Vec<String>) -> Self {
        let segments = segments
            .into_iter()
            .map(|segment| segment.replace("-", "_"))
            .collect();
        Self { segments }
    }

    pub fn iter(&self) -> std::slice::Iter<String> {
        self.segments.iter()
    }

    pub fn remove_segment(&mut self, index: usize) {
        self.segments.remove(index);
    }

    pub fn insert_segment(&mut self, index: usize, segment: String) {
        self.segments.insert(index, segment);
    }

    pub fn filename(&self) -> String {
        self.segments.iter().last().unwrap().clone()
    }
}

type Ident = Path;
type ComponentName = Path;

fn parse_components(components_value: &Value) -> BTreeMap<String, Path> {
    let mut components = BTreeMap::new();
    if let Value::Table(components_map) = components_value {
        for (ident, component_table) in components_map {
            let name_value = component_table
                .get("name")
                .expect("the `name` key does not exist.");
            if let Value::String(path) = name_value {
                let component_path = ComponentName::from_str(path);
                components.insert(ident.clone(), component_path);
            }
        }
        return components;
    } else {
        unreachable!("`components` should be a table")
    }
}

fn parse_whitelists(whitelist_value: &Value) -> BTreeMap<Ident, WhiteList> {
    let mut recorded_path = Vec::new();
    let mut whitelists = BTreeMap::new();
    if let Value::Table(whitelist_map) = whitelist_value {
        for (key, value) in whitelist_map {
            parse_whitelist_item(key, value, &mut recorded_path, &mut whitelists)
        }
        return whitelists;
    } else {
        unreachable!("whitelist should be a table")
    }
}

fn parse_whitelist_item(
    key: &str,
    value: &Value,
    recorded_path: &mut Vec<String>,
    whitelists: &mut BTreeMap<Ident, WhiteList>,
) {
    match value {
        Value::Boolean(allowed) => {
            let type_ = Ident::from_segments(recorded_path.clone());
            if whitelists.contains_key(&type_) {
                let white_list: &mut WhiteList = whitelists.get_mut(&type_).unwrap();
                white_list.add(key, *allowed);
            } else {
                let mut white_list = WhiteList::new();
                white_list.add(key, *allowed);
                whitelists.insert(type_, white_list);
            }
        }
        Value::Table(table) => {
            recorded_path.push(key.to_string());
            for (inner_key, inner_value) in table {
                parse_whitelist_item(inner_key, inner_value, recorded_path, whitelists);
            }
            recorded_path.pop();
        }
        _ => {
            unreachable!()
        }
    }
}

/// Search for the configuration file.
///
/// # Errors
///
/// Returns any unexpected filesystem error encountered when searching for the config file
pub fn lookup_conf_file() -> io::Result<Option<PathBuf>> {
    /// Possible filename to search for.
    const CONFIG_FILE_NAMES: [&str; 4] = [
        "Components.toml",
        ".Components.toml",
        "components.toml",
        ".components.toml",
    ];

    // Start looking for a config file in COMPONENT_CONFIG_DIR.(This should be the directory execute cargo component)
    let current = PathBuf::from(env::var_os("COMPONENT_CONFIG_DIR").unwrap());
    let mut found_config: Option<PathBuf> = None;

    loop {
        for config_file_name in &CONFIG_FILE_NAMES {
            if let Ok(config_file) = current.join(config_file_name).canonicalize() {
                match fs::metadata(&config_file) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                    Ok(md) if md.is_dir() => {}
                    Ok(_) => {
                        if let Some(ref found_config_) = found_config {
                            eprintln!(
                                "Using config file `{}`\nWarning: `{}` will be ignored.",
                                found_config_.display(),
                                config_file.display(),
                            );
                        } else {
                            found_config = Some(config_file);
                        }
                    }
                }
            }
        }

        if found_config.is_some() {
            return Ok(found_config);
        }

        return Ok(None);
    }
}

pub fn init(conf_path: &str) {
    let file_content = std::fs::read_to_string(conf_path).expect("Read config file failed");
    let config_toml = file_content.parse::<Value>().unwrap();
    let config = Config::parse_toml(config_toml);
    config.check_config();
    CONFIG.set(config).unwrap();
}
