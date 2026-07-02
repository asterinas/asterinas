# SPDX-License-Identifier: MPL-2.0
#
# klint, the Rust-for-Linux static-analysis driver used by the Docker image.
# It links rustc internals, so build it with the project's pinned nightly.
#
# Use the upstream Cargo.lock instead of a vendored cargoHash. The latter goes
# through fetch-cargo-vendor, whose crates.io requests currently fail without a
# User-Agent. The git dependency still needs an explicit outputHash.
#
# klint-Cargo.lock is copied from the pinned klint commit so evaluation does
# not need to fetch `src` first. Refresh it when bumping the rev.
{ makeRustPlatform, fetchFromGitHub, asterinas-rust-toolchain, sqlite
, pkg-config }:

let
  rustPlatform = makeRustPlatform {
    cargo = asterinas-rust-toolchain;
    rustc = asterinas-rust-toolchain;
  };

  src = fetchFromGitHub {
    owner = "Rust-for-Linux";
    repo = "klint";
    rev = "7d7c522b66a3e2456bc6024ff69dea619c4b9c2a";
    hash = "sha256-Yxmza8JJB4mrKYcggJfZIg7pv3l73LEQ+YMI0y1+4JI=";
  };
in rustPlatform.buildRustPackage {
  pname = "klint";
  version = "unstable-7d7c522";
  inherit src;

  cargoLock = {
    lockFile = ./klint-Cargo.lock;
    outputHashes = {
      "compiletest_rs-0.11.2" =
        "sha256-kjdqn9MggFypzB6SVWAsNqD21wZYiv+dtPvyGNi/Wqo=";
    };
  };

  # Fail with a clear message when the committed lock copy goes stale;
  # nixpkgs' own consistency check reports a misleading cargoHash error.
  postPatch = ''
    if ! diff -q ${./klint-Cargo.lock} Cargo.lock > /dev/null; then
      echo "error: nix/packages/klint-Cargo.lock does not match the pinned klint rev." >&2
      echo "Copy Cargo.lock from the new rev over nix/packages/klint-Cargo.lock." >&2
      exit 1
    fi
  '';

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ sqlite ];
  doCheck = false;
}
