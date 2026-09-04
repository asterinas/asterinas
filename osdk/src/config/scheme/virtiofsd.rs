// SPDX-License-Identifier: MPL-2.0

//! Configuration for the `virtiofsd` processes managed by OSDK.

use std::path::PathBuf;

const DEFAULT_VIRTIOFSD_PATH: &str = "/usr/libexec/virtiofsd";
const DEFAULT_SHARED_DIR: &str = "/tmp/asterinas-virtiofs";
const DEFAULT_SCRATCH_SHARED_DIR: &str = "/tmp/asterinas-virtiofs-scratch";
const DEFAULT_LOG_FILE: &str = "virtiofsd.log";
const DEFAULT_SCRATCH_LOG_FILE: &str = "virtiofsd-scratch.log";
const DEFAULT_CACHE: &str = "auto";

/// The configurable arguments for OSDK-managed `virtiofsd` processes.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VirtioFsdScheme {
    /// The `virtiofsd` executable.
    pub path: Option<PathBuf>,
    /// The shared directory for the primary virtio-fs device.
    pub shared_dir: Option<PathBuf>,
    /// The shared directory for the optional scratch virtio-fs device.
    pub scratch_shared_dir: Option<PathBuf>,
    /// The log file for the primary `virtiofsd` process.
    pub log_file: Option<PathBuf>,
    /// The log file for the optional scratch `virtiofsd` process.
    pub scratch_log_file: Option<PathBuf>,
    /// The cache mode passed to `virtiofsd`.
    pub cache: Option<String>,
    /// Whether to enable extended attributes in `virtiofsd`.
    pub xattr: Option<bool>,
    /// The sandbox mode passed to `virtiofsd`.
    pub sandbox: Option<String>,
    /// Additional arguments passed to `virtiofsd`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// The finalized arguments for OSDK-managed `virtiofsd` processes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VirtioFsd {
    pub path: PathBuf,
    pub shared_dir: PathBuf,
    pub scratch_shared_dir: PathBuf,
    pub log_file: PathBuf,
    pub scratch_log_file: PathBuf,
    pub cache: String,
    pub xattr: bool,
    pub sandbox: Option<String>,
    pub args: Vec<String>,
}

impl Default for VirtioFsd {
    fn default() -> Self {
        VirtioFsdScheme::default().finalize()
    }
}

impl VirtioFsdScheme {
    pub fn inherit(&mut self, from: &Self) {
        if self.path.is_none() {
            self.path.clone_from(&from.path);
        }
        if self.shared_dir.is_none() {
            self.shared_dir.clone_from(&from.shared_dir);
        }
        if self.scratch_shared_dir.is_none() {
            self.scratch_shared_dir.clone_from(&from.scratch_shared_dir);
        }
        if self.log_file.is_none() {
            self.log_file.clone_from(&from.log_file);
        }
        if self.scratch_log_file.is_none() {
            self.scratch_log_file.clone_from(&from.scratch_log_file);
        }
        if self.cache.is_none() {
            self.cache.clone_from(&from.cache);
        }
        if self.xattr.is_none() {
            self.xattr = from.xattr;
        }
        if self.sandbox.is_none() {
            self.sandbox.clone_from(&from.sandbox);
        }
        self.args = {
            let mut args = from.args.clone();
            args.append(&mut self.args);
            args
        };
    }

    pub fn finalize(self) -> VirtioFsd {
        VirtioFsd {
            path: self
                .path
                .unwrap_or_else(|| PathBuf::from(DEFAULT_VIRTIOFSD_PATH)),
            shared_dir: self
                .shared_dir
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SHARED_DIR)),
            scratch_shared_dir: self
                .scratch_shared_dir
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SCRATCH_SHARED_DIR)),
            log_file: self
                .log_file
                .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_FILE)),
            scratch_log_file: self
                .scratch_log_file
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SCRATCH_LOG_FILE)),
            cache: self.cache.unwrap_or_else(|| DEFAULT_CACHE.to_string()),
            xattr: self.xattr.unwrap_or(false),
            sandbox: self.sandbox,
            args: self.args,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_uses_virtiofsd_defaults() {
        let virtiofsd = VirtioFsdScheme::default().finalize();

        assert_eq!(virtiofsd.path, PathBuf::from(DEFAULT_VIRTIOFSD_PATH));
        assert_eq!(virtiofsd.shared_dir, PathBuf::from(DEFAULT_SHARED_DIR));
        assert_eq!(
            virtiofsd.scratch_shared_dir,
            PathBuf::from(DEFAULT_SCRATCH_SHARED_DIR)
        );
        assert_eq!(virtiofsd.log_file, PathBuf::from(DEFAULT_LOG_FILE));
        assert_eq!(
            virtiofsd.scratch_log_file,
            PathBuf::from(DEFAULT_SCRATCH_LOG_FILE)
        );
        assert_eq!(virtiofsd.cache, DEFAULT_CACHE);
        assert!(!virtiofsd.xattr);
        assert!(virtiofsd.sandbox.is_none());
        assert!(virtiofsd.args.is_empty());
    }
}
