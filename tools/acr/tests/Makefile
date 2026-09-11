# SPDX-License-Identifier: MPL-2.0

.PHONY: test

# Run as the `tests` package so tests/__init__.py isolates the environment.
test:
	cd .. && python -P src/_bootstrap.py unittest discover -s tests -t . -v
