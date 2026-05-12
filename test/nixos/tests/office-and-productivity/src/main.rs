// SPDX-License-Identifier: MPL-2.0

//! The test suite for office and productivity applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Document Viewers - MuPDF
// ============================================================================

#[nixos_test]
fn mutool_inspect_convert_pdf(nixos_shell: &mut Session) -> Result<(), Error> {
    // Get a sample PDF file for testing
    nixos_shell.run_cmd("curl -o /tmp/sample.pdf https://pdfobject.com/pdf/sample.pdf")?;

    // Show information about pdf resources
    nixos_shell.run_cmd_and_expect("mutool info /tmp/sample.pdf", "Info object")?;

    // Convert text from pdf
    nixos_shell.run_cmd("mutool draw -F text -o /tmp/output.txt /tmp/sample.pdf")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/output.txt", "Sample PDF")?;

    // Convert images from pdf
    nixos_shell.run_cmd("mutool draw -F png -o /tmp/page-%03d.png /tmp/sample.pdf")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/page-001.png", "page-001.png")?;

    Ok(())
}

// ============================================================================
// Document Conversion - pandoc
// ============================================================================

#[nixos_test]
fn pandoc_convert_markdown_to_html(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '# Hello World\n\nThis is a test.' > /tmp/test.md"##)?;
    nixos_shell.run_cmd("pandoc /tmp/test.md -o /tmp/test.html")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/test.html", "<h1")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/test.html", "<p>This is a test.</p>")?;
    Ok(())
}

#[nixos_test]
fn pandoc_convert_markdown_to_docx(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(r##"echo -e '# Test Document\n\nThis is content.' > /tmp/test2.md"##)?;
    nixos_shell.run_cmd("pandoc /tmp/test2.md -o /tmp/test2.docx")?;
    nixos_shell.run_cmd_and_expect("ls /tmp/test2.docx", "test2.docx")?;
    Ok(())
}

#[nixos_test]
fn pandoc_convert_html_to_markdown(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "echo '<html><body><h1>Title</h1><p>Paragraph</p></body></html>' > /tmp/test3.html",
    )?;
    nixos_shell.run_cmd("pandoc /tmp/test3.html -f html -t markdown -o /tmp/converted.md")?;
    nixos_shell.run_cmd_and_expect("cat /tmp/converted.md", "# Title")?;
    Ok(())
}
