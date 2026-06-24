# SPDX-License-Identifier: MPL-2.0

.PHONY: build
build: mermaid.min.js mermaid-init.js
	@mdbook build

mermaid.min.js mermaid-init.js:
	@mdbook-mermaid install

.PHONY: check
check: check_rust_api_doc_version check_invalid_or_broken_links

.PHONY: check_rust_api_doc_version
check_rust_api_doc_version:
	@./check_rust_api_doc_version.sh

.PHONY: check_invalid_or_broken_links
check_invalid_or_broken_links:
	@./check_invalid_or_broken_links.sh
