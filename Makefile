.PHONY: all build clean docs fmt test 

all: build test

build:
	@cd src && cargo build

test: build
	@cd src && cargo test

docs:
	@cd src && cargo doc
	@echo "" 								# Add a blank line
	@cd docs && mdbook build

fmt:
	@cd src && cargo fmt

clean:
	@cd src && cargo clean
	@cd docs && mdbook clean