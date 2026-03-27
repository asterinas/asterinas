// SPDX-License-Identifier: MPL-2.0

//! The test suite for AI and machine learning applications on Asterinas NixOS.
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
// Deep Learning Frameworks - PyTorch
// ============================================================================

#[nixos_test]
fn run_pytorch(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("python /tmp/test_torch.py", "OK")?;
    Ok(())
}

// ============================================================================
// Deep Learning Frameworks - TensorFlow
// ============================================================================

#[nixos_test]
fn run_tensorflow(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("python /tmp/test_tensorflow.py", "OK")?;
    Ok(())
}

// ============================================================================
// LLM Inference Engines - Ollama
// ============================================================================

#[nixos_test]
fn ollama_server_start(nixos_shell: &mut Session) -> Result<(), Error> {
    // Start ollama server in background
    nixos_shell.run_cmd("ollama serve > /tmp/ollama.log 2>&1 &")?;
    nixos_shell.run_cmd("sleep 3")?;

    // List models
    nixos_shell.run_cmd_and_expect("ollama list", "NAME")?;

    // Stop the server
    nixos_shell.run_cmd("pkill ollama")?;
    Ok(())
}
