// SPDX-License-Identifier: MPL-2.0

pub mod mnt_namespace;

use crate::{prelude::*, process::namespaces::mnt_namespace::MntNamespace};
pub struct Namespaces {
    mnt_ns: Arc<MntNamespace>,
}

impl Default for Namespaces {
    fn default() -> Self {
        Self {
            mnt_ns: Arc::new(MntNamespace::default()),
        }
    }
}

impl Namespaces {
    pub fn new(mnt_ns: Arc<MntNamespace>) -> Self {
        Self { mnt_ns }
    }

    pub fn mnt_ns(&self) -> &Arc<MntNamespace> {
        &self.mnt_ns
    }

    /// Reset the namespaces of the process.
    pub fn reset_namespaces(&mut self, namespaces: &Arc<Mutex<Namespaces>>) {
        let new_namespaces = namespaces.lock();
        self.mnt_ns = new_namespaces.mnt_ns().clone();
    }
}
