// SPDX-License-Identifier: MPL-2.0

//! The test suite for monitoring and observability applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Metrics & Alerting - Prometheus
// ============================================================================

#[nixos_test]
fn prometheus_query_metrics(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.with_background_process(
        BackgroundProcess::new(
            "prometheus --config.file=/tmp/prometheus.yml --web.listen-address='10.0.2.15:9090' > /tmp/prometheus.log 2>&1 &",
            CommandCheck::new("curl -s http://10.0.2.15:9090/-/healthy", "Prometheus"),
            "pkill prometheus",
            CommandCheck::new("! pgrep -x prometheus >/dev/null && echo stopped", "stopped"),
        ),
        |shell| {
            shell.run_cmd_and_expect(
                "curl -s 'http://10.0.2.15:9090/api/v1/query?query=up'",
                "success",
            )?;
            shell.run_cmd_and_expect(
                "curl -s http://10.0.2.15:9090/api/v1/targets",
                "success",
            )
        },
    )?;

    Ok(())
}
