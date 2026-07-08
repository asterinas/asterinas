// SPDX-License-Identifier: MPL-2.0

use super::label::SmackLabel;
use crate::{fs::vfs::file_system::FileSystem, prelude::*};

/// Smack label state attached to a mounted filesystem.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SmackMountLabels {
    default_label: SmackLabel,
    root_label: Option<SmackLabel>,
    transmute_label: Option<SmackLabel>,
    // Linux accepts `smackfsfloor`, but the option is not enforced.
    floor_label: Option<SmackLabel>,
    // Linux accepts `smackfshat`, but the option is not enforced.
    hat_label: Option<SmackLabel>,
}

impl SmackMountLabels {
    /// Parses Smack mount labels from Linux-compatible mount options.
    pub fn parse(args: Option<&CStr>) -> Result<(Self, bool)> {
        let mut labels = Self::default();
        let Some(args) = args else {
            return Ok((labels, false));
        };

        let mut found_smack_option = false;
        let args = args.to_string_lossy();
        for token in args
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            let Some((name, value)) = token.split_once('=') else {
                continue;
            };

            let parsed_label = match name {
                "smackfsdef" | "smackfsdefault" => &mut labels.default_label,
                "smackfsroot" => {
                    found_smack_option = true;
                    labels.root_label = Some(SmackLabel::parse(value)?);
                    continue;
                }
                "smackfstransmute" => {
                    found_smack_option = true;
                    labels.transmute_label = Some(SmackLabel::parse(value)?);
                    continue;
                }
                "smackfsfloor" => {
                    found_smack_option = true;
                    labels.floor_label = Some(SmackLabel::parse(value)?);
                    continue;
                }
                "smackfshat" => {
                    found_smack_option = true;
                    labels.hat_label = Some(SmackLabel::parse(value)?);
                    continue;
                }
                _ => continue,
            };

            found_smack_option = true;
            *parsed_label = SmackLabel::parse(value)?;
        }

        Ok((labels, found_smack_option))
    }

    /// Returns the label used for filesystem objects that lack a Smack xattr.
    pub fn default_label(&self) -> SmackLabel {
        self.default_label.clone()
    }

    /// Returns the label to assign to the filesystem root, if specified.
    pub fn root_label(&self) -> Option<&SmackLabel> {
        self.transmute_label.as_ref().or(self.root_label.as_ref())
    }

    /// Returns whether the root should be marked transmuting.
    pub fn root_is_transmuting(&self) -> bool {
        self.transmute_label.is_some()
    }
}

/// Applies root-specific Smack mount labels to a filesystem.
pub fn apply_root_labels(fs: &dyn FileSystem) -> Result<()> {
    let root_inode = fs.root_inode();
    let labels = fs.sb().smack;

    if let Some(root_label) = labels.root_label() {
        super::xattr::set_access_label(root_inode.as_ref(), root_label)?;
    }
    if labels.root_is_transmuting() {
        super::xattr::set_transmute(root_inode.as_ref())?;
    }

    Ok(())
}
