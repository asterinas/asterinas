# SPDX-License-Identifier: MPL-2.0

# Tests for the aster-code-review deterministic scripts.
#
# Suites are auto-discovered (test_*.sh):
# add a suite by adding a file, nothing here changes.
# This Makefile owns all test detail;
# the package root only delegates to it (`make -C tests`).
#
# Targets (default runs all suites):
#   make                run every suite; prints per-case ok/FAIL, non-zero if any fail
#   make test_<suite>   run one suite, e.g. `make test_resolve_target`
#   make help           list the auto-discovered suites
#
# Each suite (test_*.sh) has a shebang and is executable,
# so it runs directly; lib.sh is sourced by the suites, not run.
SUITES := $(wildcard test_*.sh)

.PHONY: test help

# Default goal: run every suite, aggregating failures (all run; non-zero if any fail).
test:
	@fail=0; \
	 for s in $(SUITES); do ./$$s || fail=1; done; \
	 echo "----"; \
	 if [ $$fail -eq 0 ]; then echo "ALL SUITES PASSED"; else echo "SOME SUITES FAILED"; fi; \
	 exit $$fail

# Run a single suite: `make test_resolve_target` -> ./test_resolve_target.sh
test_%:
	@./test_$*.sh

# List the auto-discovered suites (their bare `test_<suite>` target names).
help:
	@echo "suites (run one with 'make <target>', all with 'make'):"
	@for s in $(SUITES); do echo "  $${s%.sh}"; done
