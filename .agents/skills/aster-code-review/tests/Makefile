# Tests for the aster-code-review deterministic scripts.
#
# Suites are auto-discovered (test-*.sh):
# add a suite by adding a file, nothing here changes.
# This Makefile owns all test detail;
# the package root only delegates to it (`make -C tests`).
#
# Targets (default runs all suites):
#   make                run every suite; prints per-case ok/FAIL, non-zero if any fail
#   make test-<suite>   run one suite, e.g. `make test-resolve-target`
#
# Each suite (test-*.sh) has a shebang and is executable,
# so it runs directly; lib.sh is sourced by the suites, not run.
SUITES := $(wildcard test-*.sh)

.PHONY: test

# Default goal: run every suite, aggregating failures (all run; non-zero if any fail).
test:
	@fail=0; \
	 for s in $(SUITES); do ./$$s || fail=1; done; \
	 echo "----"; \
	 if [ $$fail -eq 0 ]; then echo "ALL SUITES PASSED"; else echo "SOME SUITES FAILED"; fi; \
	 exit $$fail

# Run a single suite: `make test-resolve-target` -> ./test-resolve-target.sh
test-%:
	@./test-$*.sh
