.PHONY: all build clean docs fmt run setup test tools eval

all: build

setup:
	@rustup component add rust-src
	@rustup component add rustc-dev
	@rustup component add llvm-tools-preview
	@cargo install mdbook

build:
	@make --no-print-directory -C regression
	@cargo kbuild

tools:
	@cd services/libs/comp-sys && cargo install --path cargo-component

run: build
	@cargo krun

# Test Jinux in a QEMU guest VM and run a series of evaluations.
eval: build
	@cargo keval

# The usermode cargo test of Jinux frame and Jinux standard library.
test: build
	@cargo ktest

docs:
	@cargo doc 								# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

check:
	@cargo fmt --check 				# Check Rust format issues
	@cargo clippy					# Check common programming mistakes

clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean
