# CI/CD & DevOps

This category covers continuous integration/deployment tools and infrastructure automation.

## CI/CD Runners

### just

[just](https://github.com/casey/just) is a handy way to save and run project-specific commands.

#### Installation

```nix
environment.systemPackages = [ pkgs.just ];
```

#### Verified Usage

```bash
# List recipes
just --list

# Run a recipe
just build
```

### Task

[Task](https://taskfile.dev/) is a fast, cross-platform build tool inspired by Make, designed for modern workflows.

#### Installation

```nix
environment.systemPackages = [ pkgs.go-task ];
```

#### Verified Usage

```bash
# List tasks
task --list-all

# Run a task named build
task build
```

## Release Automation

### GoReleaser

[GoReleaser](https://goreleaser.com/) does everything you need to create a professional release process for Go, Rust, TypeScript, Zig, and Python projects.

#### Installation

```nix
environment.systemPackages = [ pkgs.goreleaser ];
```

#### Verified Usage

```bash
# Check configuration
goreleaser check

# Run a snapshot release locally
goreleaser release --snapshot --clean
```
