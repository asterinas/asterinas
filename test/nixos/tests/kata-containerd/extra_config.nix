{ config, lib, pkgs, ... }:

let
  qemuWrapper = pkgs.writeShellScriptBin "kata-qemu-wrapper" ''
    out_dir="''${KATA_DEBUG_OUT:-/tmp/kata-debug-out}"
    mkdir -p "$out_dir" 2>/dev/null || out_dir=/tmp
    stamp=$(${pkgs.coreutils}/bin/date +%s 2>/dev/null || echo now)
    base="$out_dir/qemu-wrapper-$stamp-$$"

    args=()
    for arg in "$@"; do
      case "$arg" in
        *accel=kvm*)
          args+=("''${arg/accel=kvm/accel=tcg}")
          ;;
        host|host,*)
          args+=("max")
          ;;
        *)
          args+=("$arg")
          ;;
      esac
    done

    {
      printf '%s\n' "${pkgs.qemu_test}/bin/qemu-system-x86_64"
      for arg in "$@"; do
        printf '%s\n' "$arg"
      done
    } > "$base.argv" 2>/dev/null || true
    {
      printf '%s\n' "${pkgs.qemu_test}/bin/qemu-system-x86_64"
      for arg in "''${args[@]}"; do
        printf '%s\n' "$arg"
      done
    } > "$base.effective-argv" 2>/dev/null || true
    exec ${pkgs.qemu_test}/bin/qemu-system-x86_64 "''${args[@]}" 2>"$base.stderr"
  '';

  kataRuntimeRs = pkgs.rustPlatform.buildRustPackage rec {
    pname = "kata-runtime-rs";
    version = pkgs.kata-runtime.version;

    src = pkgs.kata-runtime.src;
    sourceRoot = "${src.name}/src/runtime-rs";

    cargoLock = {
      lockFile = "${src}/src/runtime-rs/Cargo.lock";
      outputHashes = {
        "api_client-0.1.0" = "sha256-aWtVgYlcbssL7lQfMFGJah8DrJN0s/w1ZFncCPHT1aE=";
      };
    };

    nativeBuildInputs = with pkgs; [
      pkg-config
      protobuf
    ];

    buildInputs = with pkgs; [
      openssl
      systemd
      zlib
    ];

    postPatch = ''
      chmod -R u+w ..
      substituteInPlace crates/runtimes/virt_container/Cargo.toml \
        --replace 'default = ["cloud-hypervisor"]' \
                  'default = []'
    '';

    preBuild = ''
      make static-checks-build \
        HYPERVISOR=qemu \
        USE_BUILDIN_DB=false \
        DBCMD= \
        CLHCMD= \
        FCCMD= \
        REMOTE= \
        PREFIX=/run/current-system/sw \
        QEMUBINDIR=/run/current-system/sw/bin \
        LIBEXECDIR=/run/current-system/sw/bin
    '';

    cargoBuildFlags = [
      "-p"
      "shim"
      "--bin"
      "containerd-shim-kata-v2"
    ];

    doCheck = false;

    installPhase = ''
      runHook preInstall

      shim_bin=$(find target -type f -path '*/release/containerd-shim-kata-v2' | head -n1)
      install -Dm755 "$shim_bin" "$out/bin/containerd-shim-kata-v2"

      install -Dm644 \
        config/configuration-qemu-runtime-rs.toml \
        "$out/share/defaults/kata-containers/runtime-rs/configuration-qemu-runtime-rs.toml"
      cfg="$out/share/defaults/kata-containers/runtime-rs/configuration-qemu-runtime-rs.toml"
      awk '
        /^\[agent\.kata\]/ { section = "agent" }
        /^\[hypervisor\.qemu\]/ { section = "qemu" }
        /^\[runtime\]/ { section = "runtime" }
        /^\[/ && !/^\[agent\.kata\]/ && !/^\[hypervisor\.qemu\]/ && !/^\[runtime\]/ { section = "" }
        section == "qemu" && /^path = / {
          print "path = \"${qemuWrapper}/bin/kata-qemu-wrapper\""
          next
        }
        section == "qemu" && /^valid_hypervisor_paths = / {
          print "valid_hypervisor_paths = [\"${qemuWrapper}/bin/kata-qemu-wrapper\"]"
          next
        }
        { print }
      ' "$cfg" > "$cfg.tmp"
      mv "$cfg.tmp" "$cfg"

      ln -s configuration-qemu-runtime-rs.toml \
        "$out/share/defaults/kata-containers/runtime-rs/configuration.toml"

      runHook postInstall
    '';
  };
in

{
  hardware.enableRedistributableFirmware = lib.mkForce false;

  virtualisation.containerd = {
    enable = true;
    settings = {
      version = 2;
      plugins."io.containerd.grpc.v1.cri".containerd.runtimes.kata = {
        runtime_type = "io.containerd.kata.v2";
      };
    };
  };

  environment.systemPackages = with pkgs; [
    kataRuntimeRs
    kata-runtime.passthru.kata-images
    qemuWrapper
    qemu_test
    runc
    socat
    virtiofsd
    (writeShellScriptBin "kata-debug-check" ''
      exec sh /etc/kata-debug/guest/00-check-kata-env.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-start-containerd" ''
      exec sh /etc/kata-debug/guest/01-start-containerd-debug.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-run-rootfs" ''
      exec sh /etc/kata-debug/guest/02-run-rootfs-probe.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-watch-serial" ''
      exec sh /etc/kata-debug/guest/03-watch-qemu-serial.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-collect" ''
      exec sh /etc/kata-debug/guest/04-collect-logs.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-run-all" ''
      exec sh /etc/kata-debug/guest/10-run-all.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-stop" ''
      exec sh /etc/kata-debug/guest/99-stop-containerd-debug.sh "$@"
    '')
  ];

  environment.pathsToLink = [
    "/share/kata-containers"
  ];

  environment.etc."kata-containers/configuration.toml".source =
    "${kataRuntimeRs}/share/defaults/kata-containers/runtime-rs/configuration.toml";

  environment.etc."kata-debug/guest".source = extraFileDir + "/guest-scripts";

  systemd.services.containerd = {
    environment.KATA_CONF_FILE = "/etc/kata-containers/configuration.toml";
    path = [
      kataRuntimeRs
      pkgs.qemu_test
      pkgs.virtiofsd
    ];
  };
}
