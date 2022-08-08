.PHONY: all build clean docs fmt test 

all: build test

build:
	@cd src && cargo build

test: build
	@cd src && cargo test

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