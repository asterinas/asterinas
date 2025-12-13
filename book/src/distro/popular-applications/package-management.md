# Package Management

NixOS provides a set of [tools](https://nix.dev/manual/nix/2.28/command-ref/main-commands)
for building, installing, and managing packages.

## Verified Usage

### `nix-build`

`nix-build` builds Nix derivations and produces outputs in the Nix store.
It is the preferred way to build software reproducibly.

```bash
# Step 1: Create a clean workspace
rm -rf /tmp/nix-hello-c && mkdir -p /tmp/nix-hello-c && cd /tmp/nix-hello-c

# Step 2: Write a C program
cat > hello.c <<'EOF'
#include <stdio.h>

int main(void) {
    puts("Hello, World!");
    return 0;
}
EOF

# Step 3: Write a default.nix
cat > default.nix <<'EOF'
{ pkgs ? import <nixpkgs> {} }:

pkgs.stdenv.mkDerivation {
  pname = "hello-c";
  version = "0.1.0";
  src = ./.;

  dontConfigure = true;

  buildPhase = ''
    cc hello.c -o hello
  '';

  installPhase = ''
    mkdir -p $out/bin
    install -m755 hello $out/bin/hello
  '';
}
EOF

# Step 4: Build and run
nix-build
./result/bin/hello
```

### `nix-env`

`nix-env` installs or removes individual packages in your user profile.

```bash
# Install the `hello` package
nix-env -iA nixos.hello

# Remove the `hello` package
nix-env -e hello
```

### `nix-shell`

`nix-shell` creates a temporary development environment with the specified dependencies.
This is useful for testing software without modifying your system environment.

```bash
# Enter a shell with the `hello` package available
nix-shell -p hello
```

### `nixos-rebuild`

`nixos-rebuild` manages the entire system configuration declaratively.
It applies changes defined in `configuration.nix`,
and is the recommended approach for installing packages system-wide.

```bash
# Edit the system configuration file
vim /etc/nixos/configuration.nix

# Apply configuration changes and rebuild the system without rebooting
nixos-rebuild test
```

<!--
TODO: upgrade mdbook to enable admonition blocks like the one below:

> [!WARNING]
> `nixos-rebuild switch` is not yet supported
-->
