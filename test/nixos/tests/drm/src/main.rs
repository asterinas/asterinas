// SPDX-License-Identifier: MPL-2.0

//! Smoke tests for the DRM, Xorg, and Mesa integration path on Asterinas NixOS.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn drm_xorg_mesa_smoke(nixos_shell: &mut Session) -> Result<(), Error> {
    // The guest-side script verifies the complete DRM → Xorg → Mesa path.
    nixos_shell.run_cmd_and_expect("/tmp/drm_xorg_mesa_smoke.sh", "DRM_XORG_MESA_SMOKE_OK")?;

    Ok(())
}
