.PHONY: all build clean docs fmt run setup test

all: build test

setup:
	@rustup component add rust-src
	@rustup component add llvm-tools-preview
	@cargo install mdbook

build:
	@cd src && cargo kbuild
	@cd src && cargo kimage

run: build
	@cd src && cargo krun

test: build
	@#cd src && cargo ktest

docs:
	@cd src && cargo doc 					# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

check:
	@cd src && cargo check					# Check dependency errors
	@cd src && cargo fmt --check 			# Check Rust format issues
	@cd src && cargo clippy					# Check common programming mistakes

clean:
	@cd src && cargo clean
	@cd docs && mdbook clean
