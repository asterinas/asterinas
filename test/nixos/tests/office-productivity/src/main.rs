// SPDX-License-Identifier: MPL-2.0

//! The test suite for office and productivity applications on Asterinas NixOS.
//!
//! # Document maintenance
//!
//! An application's test suite and its "Verified Usage" section in Asterinas Book
//! should always be kept in sync.
//! So whenever you modify the test suite,
//! review the documentation and see if should be updated accordingly.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Document Viewers - MuPDF
// ============================================================================

#[nixos_test]
fn mupdf_info_draw(nixos_shell: &mut Session) -> Result<(), Error> {
    // Get a simple PDF file for testing
    nixos_shell.run_cmd("curl -o /tmp/dummy.pdf https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf")?;

    // Show information about pdf resources
    nixos_shell.run_cmd_and_expect("mutool info /tmp/dummy.pdf", "Info object")?;

    // Convert text from pdf
    nixos_shell.run_cmd("mutool draw -F text -o /tmp/output.txt /tmp/dummy.pdf")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/output.txt", "Dummy PDF file")?;

    // Convert images from pdf
    nixos_shell.run_cmd("mutool draw -F png -o /tmp/page-%03d.png /tmp/dummy.pdf")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/page-001.png", "page-001.png")?;

    Ok(())
}

// ============================================================================
// Document Conversion - Pandoc
// ============================================================================

#[nixos_test]
fn pandoc_md_to_html(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '# Hello World\n\nThis is a test.' > /tmp/test.md"##)?;
    nixos_shell.run_cmd("pandoc /tmp/test.md -o /tmp/test.html")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/test.html", "<h1")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/test.html", "<p>This is a test.</p>")?;
    Ok(())
}

#[nixos_test]
fn pandoc_md_to_docx(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '# Test Document\n\nThis is content.' > /tmp/test2.md"##)?;
    nixos_shell.run_cmd("pandoc /tmp/test2.md -o /tmp/test2.docx")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/test2.docx", "test2.docx")?;
    Ok(())
}

#[nixos_test]
fn pandoc_html_to_md(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "echo '<html><body><h1>Title</h1><p>Paragraph</p></body></html>' > /tmp/test3.html",
    )?;
    nixos_shell.run_cmd("pandoc /tmp/test3.html -f html -t markdown -o /tmp/converted.md")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/converted.md", "# Title")?;
    Ok(())
}
