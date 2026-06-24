{ config, lib, pkgs, ... }:

let
  busyboxRoot = pkgs.runCommand "kata-busybox-root" { } ''
    mkdir -p "$out/bin"
    cp ${pkgs.pkgsStatic.busybox}/bin/busybox "$out/bin/busybox"
    chmod 0755 "$out/bin/busybox"
    ln -s busybox "$out/bin/sh"
  '';

  busyboxImage = pkgs.dockerTools.buildImage {
    name = "busybox";
    tag = "latest";
    copyToRoot = busyboxRoot;
    config = {
      Cmd = [ "/bin/sh" ];
    };
  };

  registryProxy = "http://10.0.2.2:18089";
  registryNoProxy = "localhost,127.0.0.1,::1,10.0.2.2";
  registryPullImages = "docker.io/library/busybox:latest docker.m.daocloud.io/library/busybox:latest";

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
        section == "qemu" && /^#?default_vcpus = / {
          print "default_vcpus = 1"
          next
        }
        section == "qemu" && /^#?default_maxvcpus = / {
          print "default_maxvcpus = 1"
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
    curl
    iproute2
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
    (writeShellScriptBin "kata-debug-run-image-probe" ''
      exec sh /etc/kata-debug/guest/20-run-image-probe.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-run-image" ''
      exec sh /etc/kata-debug/guest/11-run-image-all.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-pull-image" ''
      exec sh /etc/kata-debug/guest/21-pull-image.sh "$@"
    '')
    (writeShellScriptBin "kata-debug-stop" ''
      exec sh /etc/kata-debug/guest/99-stop-containerd-debug.sh "$@"
    '')
  ];

  environment.variables = {
    HTTP_PROXY = registryProxy;
    HTTPS_PROXY = registryProxy;
    ALL_PROXY = registryProxy;
    KATA_PULL_IMAGES = registryPullImages;
    NO_PROXY = registryNoProxy;
    http_proxy = registryProxy;
    https_proxy = registryProxy;
    all_proxy = registryProxy;
    no_proxy = registryNoProxy;
  };

  environment.pathsToLink = [
    "/share/kata-containers"
  ];

  environment.etc."kata-containers/configuration.toml".source =
    "${kataRuntimeRs}/share/defaults/kata-containers/runtime-rs/configuration.toml";

  environment.etc."kata-debug/guest".source = extraFileDir + "/guest-scripts";
  environment.etc."kata-debug/busybox.tar".source = busyboxImage;

  systemd.services.containerd = {
    environment = {
      KATA_CONF_FILE = "/etc/kata-containers/configuration.toml";
      HTTP_PROXY = registryProxy;
      HTTPS_PROXY = registryProxy;
      ALL_PROXY = registryProxy;
      KATA_PULL_IMAGES = registryPullImages;
      NO_PROXY = registryNoProxy;
      http_proxy = registryProxy;
      https_proxy = registryProxy;
      all_proxy = registryProxy;
      no_proxy = registryNoProxy;
    };
    path = [
      kataRuntimeRs
      pkgs.qemu_test
      pkgs.virtiofsd
    ];
  };
}
