// SPDX-License-Identifier: MPL-2.0

//! To add a new case:
//! 2. Add a submodule in a current case crate, and apply `#[short_vis_path::add(...)]`.
//! 3. Add a test below that runs `cargo expand` for the case,
//!    and asserts the expected expanded visibility paths.

use std::process::Command;

/// The information of a testing project.
struct TestCase {
    project_dir: String,
    cargo_expand_stdout: String,
}

impl TestCase {
    fn new(project_dir: &str) -> Self {
        let output = Command::new("cargo")
            .arg("expand")
            .current_dir(project_dir)
            .output()
            .unwrap();

        TestCase {
            project_dir: project_dir.to_owned(),
            cargo_expand_stdout: String::from_utf8(output.stdout).unwrap(),
        }
    }

    fn assert_contains(&self, text: &str) {
        let dir = &self.project_dir;
        let stdout = &self.cargo_expand_stdout;
        assert!(
            stdout.contains(text),
            "[project: {dir}] expected code `{text}` should be in the macro expansion:\n`{stdout}`"
        );
    }
}

// Verifies default resolution, path overrides, and attribute placement.
#[test]
fn syntax_and_behavior() {
    let case = TestCase::new("./tests/syntax-and-behavior");
    let expected = [
        // src/test_deepest_module_wins/parent/child/parent/child.rs
        "pub(in crate::test_deepest_module_wins::parent::child::parent) const fn deepest_wins",
        // src/test_override/parent/child/parent/child.rs
        "pub(in crate::test_override::parent) const fn override_parent",
        // src/test_mod_rs_flavor/child/mod.rs
        "pub(in crate::test_mod_rs_flavor) type RecognizeModRs",
        // src/test_multiple_arguments/one_ident_and_one_override.rs
        "pub(in crate::test_multiple_arguments) type VisibleToParent",
        "pub(in crate::test_multiple_arguments) type VisibleToParentToo",
        // src/test_multiple_arguments/two_idents.rs
        "pub(in crate::test_multiple_arguments) type VisibleToParent",
        "pub(in crate::test_multiple_arguments::two_idents) type VisibleToCurrent",
        // src/lib.rs
        "pub(in crate::test_outer_attribute) type VisibleToParent",
    ];

    for text in expected {
        case.assert_contains(text);
    }
}
