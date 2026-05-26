// SPDX-License-Identifier: MPL-2.0

//! The test suite for AI and machine learning applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Deep Learning Frameworks - PyTorch
// ============================================================================

#[nixos_test]
fn pytorch_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "python3 /tmp/test_pytorch.py && echo __PYTORCH_OK__",
        "__PYTORCH_OK__",
    )?;
    Ok(())
}

// ============================================================================
// Deep Learning Frameworks - TensorFlow
// ============================================================================

#[nixos_test]
fn tensorflow_run_script(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect(
        "python3 /tmp/test_tensorflow.py && echo __TENSORFLOW_OK__",
        "__TENSORFLOW_OK__",
    )?;
    Ok(())
}

// ============================================================================
// LLM Inference Engines - Ollama
// ============================================================================

#[nixos_test]
fn ollama_start_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "ollama serve > /tmp/ollama.log 2>&1 &",
            CommandCheck::new("ollama list", "NAME"),
            "pkill ollama",
            CommandCheck::new("! pgrep -x ollama >/dev/null && echo stopped", "stopped"),
        ),
        |shell| shell.run_cmd_and_expect("ollama list", "NAME"),
    )?;

    Ok(())
}

// ============================================================================
// AI Coding Agents - Codex
// ============================================================================

#[nixos_test]
fn codex_show_help(nixos_shell: &mut Session) -> Result<(), Error> {
    // CI does not provide a Codex API key, so we cannot test real requests or
    // interactive workflows. This smoke test only verifies that the Codex
    // package is installed and that its offline CLI entrypoint works.
    nixos_shell.run_cmd_and_expect("codex --help", "Run Codex non-interactively")?;
    Ok(())
}
