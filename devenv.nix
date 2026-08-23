{
  inputs,
  lib,
  multiverse,
  pkgs,
  ...
}:

let
  python = pkgs.python3.withPackages (pythonPackages: [
    pythonPackages.pyyaml
    pythonPackages.yq
  ]);

  pinnedTypos = pkgs.writeShellScriptBin "typos" ''
    # The historical binary uses an older glibc. Host-injected libraries may
    # come from the current system and are not ABI-compatible with it.
    unset LD_PRELOAD
    exec ${multiverse.typos."1.39.0"}/bin/typos "$@"
  '';

  pinnedPackages = [
    multiverse.qemu."10.2.1"
    multiverse.cargo-expand."1.0.122"
    multiverse.cargo-udeps."0.1.61"
    multiverse.lychee."0.24.2"
    multiverse.mdbook-mermaid."0.17.0"
    multiverse.mdbook."0.5.2"
    pinnedTypos
  ];

  commonPackages = with pkgs; [
    bash
    cachix
    cargo-binutils
    clang
    clang-tools
    cpio
    curl
    dosfstools
    e2fsprogs
    exfatprogs
    file
    gcc
    gdb
    git
    gnumake
    jq
    mtools
    nix
    nixfmt-rfc-style
    parted
    pkg-config
    python
    socat
    strace
    unzip
    wget
    xorriso
    zip
  ];

  linuxPackages = with pkgs; [
    bridge-utils
    cpuid
    grub2
    iproute2
    iptables
    nettools
    openssh
    virtiofsd
  ];
in
{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;

    # Match the existing rustup-based container instead of injecting a host
    # linker into kernel and bare-metal target builds.
    clangLinker.enable = false;
    lsp.package = pkgs.rust-analyzer;
  };

  packages = commonPackages ++ pinnedPackages ++ lib.optionals pkgs.stdenv.isLinux linuxPackages;

  env = {
    VDSO_LIBRARY_DIR = inputs.linux-vdso;
  };

  scripts = {
    sctrace = {
      description = "Trace a command and check its system calls against SCML rules";
      exec = ''
        exec "$DEVENV_ROOT/tools/sctrace.sh" "$@"
      '';
    };

    asterinas-env-check = {
      description = "Check the essential Asterinas development tools";
      exec = ''
        set -eu

        rustc --version
        cargo --version
        rust-objcopy --version | head -n 1
        qemu-system-x86_64 --version | head -n 1
        qemu-system-riscv64 --version | head -n 1
        qemu-system-loongarch64 --version | head -n 1
        grub-mkrescue --version
        yq --version
        nixfmt --version
        typos --version
        test -f "$VDSO_LIBRARY_DIR/vdso_x86_64.so"
      '';
    };
  };

  tasks = {
    "asterinas:test" = {
      description = "Run user-mode Rust tests";
      exec = "make test";
    };

    "asterinas:check" = {
      description = "Run the repository checks";
      exec = "make check";
    };
  };

  enterShell = ''
    export PATH="$DEVENV_ROOT/target/bin:$PATH"
    ASTER_SCML="$(find "$DEVENV_ROOT/book/src/kernel/linux-compatibility" -name '*.scml' -print | tr '\n' ' ')"
    export ASTER_SCML
  '';
}
