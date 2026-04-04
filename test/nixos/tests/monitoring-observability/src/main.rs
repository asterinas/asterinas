// SPDX-License-Identifier: MPL-2.0

//! The test suite for monitoring and observability applications on Asterinas NixOS.
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
// Metrics & Alerting - Prometheus
// ============================================================================

#[nixos_test]
fn prometheus_server(nixos_shell: &mut Session) -> Result<(), Error> {
    // Start Prometheus server
    nixos_shell.run_cmd("prometheus --config.file=/tmp/prometheus.yml --web.listen-address='10.0.2.15:9090' > /tmp/prometheus.log 2>&1 &")?;
    nixos_shell.run_cmd("sleep 10")?;

    // Check if Prometheus is responding
    nixos_shell.run_cmd_and_expect("curl -s http://10.0.2.15:9090/-/healthy", "Prometheus")?;

    // Query Prometheus up metric
    nixos_shell.run_cmd_and_expect(
        "curl -s 'http://10.0.2.15:9090/api/v1/query?query=up'",
        "success",
    )?;

    // Check targets endpoint
    nixos_shell.run_cmd_and_expect("curl -s http://10.0.2.15:9090/api/v1/targets", "success")?;

    // Stop Prometheus server
    nixos_shell.run_cmd("pkill prometheus")?;
    Ok(())
}
