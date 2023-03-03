.PHONY: all build clean docs fmt run setup test tools

all: build test

setup:
	@rustup component add rust-src
	@rustup component add rustc-dev
	@rustup component add llvm-tools-preview
	@cargo install mdbook

build:
	@make --no-print-directory -C src/ramdisk
	@cd src && cargo kbuild
	@cd src && cargo kimage

tools:
	@cd src/services/comp-sys && cargo install --path cargo-component

run: build
	@cd src && cargo krun

test: build
	@cd src && cargo ktest

docs:
	@cd src && cargo doc 					# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

check:
	@cd src && cargo fmt --check 			# Check Rust format issues
	@cd src && cargo clippy					# Check common programming mistakes

clean:
	@cd src && cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C src/ramdisk clean
