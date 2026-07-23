# Patched systemdMinimal package for Asterinas NixOS.
{ pkgs }:

pkgs.systemdMinimal.overrideAttrs (old: {
  patches = (old.patches or [ ]) ++ [
    ../overlays/systemd/0001-Skip-mount-state-checking.patch
    ../overlays/systemd/0002-Disable-loop-too-fast-warning.patch
    ../overlays/systemd/0003-Switch-MS_SLAVE-to-MS_PRIVATE.patch
  ];
  postInstall = let
    extraPostInstall = ''
      mkdir -p "$out/example/systemd/system"
      for svc in systemd-logind systemd-user-sessions systemd-firstboot \
                 systemd-random-seed systemd-vconsole-setup; do
        cat > "$out/example/systemd/system/$svc.service" <<EOF
      [Unit]
      Description=placeholder ''${svc} (disabled)
      [Service]
      Type=oneshot
      ExecStart=/bin/true
      EOF
      done
      cat > "$out/example/systemd/system/dbus-org.freedesktop.login1.service" <<'EOF'
      [Unit]
      Description=placeholder dbus-org.freedesktop.login1
      [Service]
      Type=dbus
      BusName=org.freedesktop.login1
      ExecStart=/bin/true
      EOF
      cat > "$out/example/systemd/system/user@.service" <<'EOF'
      [Unit]
      Description=placeholder user@.service
      [Service]
      Type=oneshot
      RemainAfterExit=yes
      ExecStart=/bin/true
      EOF
      cat > "$out/example/systemd/system/user-runtime-dir@.service" <<'EOF'
      [Unit]
      Description=placeholder user-runtime-dir@.service
      [Service]
      Type=oneshot
      RemainAfterExit=yes
      ExecStart=/bin/mkdir -p /run/user/%i
      EOF
      cat > "$out/example/systemd/system/local-fs.target.wants/tmp.mount" <<'EOF'
      # placeholder
      EOF
      if [ ! -e "$out/lib/systemd/systemd-bsod" ]; then
        mkdir -p "$out/lib/systemd"
        cat > "$out/lib/systemd/systemd-bsod" <<'STUB'
      #!/bin/sh
      exit 0
      STUB
        chmod +x "$out/lib/systemd/systemd-bsod"
      fi
      for unit in getty@.service serial-getty@.service; do
        if [ -f "$out/example/systemd/system/$unit" ]; then
          sed -i '/^ImportCredential=/d' "$out/example/systemd/system/$unit"
        fi
      done
    '';
  in (old.postInstall or "") + "\n" + extraPostInstall;
})
