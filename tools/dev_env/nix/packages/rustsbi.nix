# SPDX-License-Identifier: MPL-2.0
#
# RustSBI Prototyper dynamic firmware for RISC-V kernel tests.
{
  lib,
  rust-bin,
  makeRustPlatform,
  symlinkJoin,
  llvmPackages,
}:

let
  # The toolchain file is needed during evaluation. Fetching it here avoids
  # building a source-fetching derivation for another host architecture.
  src = builtins.fetchTarball {
    url = "https://github.com/rustsbi/rustsbi/archive/ee559644418b29cd3be5e55c9dda4af9f87a1cae.tar.gz";
    sha256 = "sha256-XH4+GLjbmChykAHnPHukjpJnnO0FGGIzCqZG0d0Rk3I=";
  };
  # RustSBI has its own toolchain.
  toolchain = rust-bin.fromRustupToolchainFile "${src}/rust-toolchain.toml";
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
in
rustPlatform.buildRustPackage rec {
  pname = "asterinas-rustsbi";
  version = "unstable-2026-11-17";
  inherit src;

  # `build-std` also resolves the standard library's Cargo.lock. Vendor both
  # sets of dependencies so the upstream build works in Nix's offline sandbox.
  cargoDeps = symlinkJoin {
    name = "${pname}-${version}-vendor";
    paths = [
      (rustPlatform.fetchCargoVendor {
        inherit pname version src;
        hash = "sha256-YKVpZxnnmwodPOpGIipIrEws1a8100ZfP5PGGkz8948=";
      })
      (rustPlatform.fetchCargoVendor {
        name = "rustsbi-std";
        src = "${toolchain}/lib/rustlib/src/rust/library";
        hash = "sha256-QWU0JRWLl6C+XUYVwQqCccfFXtzjUgAao41vbD1bzSY=";
      })
    ];
  };
  nativeBuildInputs = [ llvmPackages.bintools ];
  # rust-lld is invoked directly and cannot accept cargo-auditable's -Wl flags.
  auditable = false;

  buildPhase = ''
    runHook preBuild
    cargo prototyper build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -D -m 644 \
      target/riscv64gc-unknown-none-elf/release/rustsbi-prototyper-dynamic.bin \
      $out/share/rustsbi/rustsbi-prototyper-dynamic.bin
    runHook postInstall
  '';

  # The firmware runs on RISC-V guests, not on the build host.
  doCheck = false;

  meta = {
    description = "RustSBI Prototyper dynamic firmware for RISC-V guests";
    homepage = "https://github.com/rustsbi/rustsbi";
    license = lib.licenses.mit;
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
}
