# Task dispatcher for the aster-code-review skill.
#
# The root only DELEGATES: each subdirectory owns its tasks in its own Makefile,
# and the root invokes the matching sub-make target.
# ACR_AGENT_PROFILE / PROBLEMS / KEEP pass through to the sub-make.
#
# Targets (default is `smoke`):
#   make  /  make smoke  ACR_AGENT_PROFILE=codex    quick agent smoke test     (-> benchmark/)
#   make benchmark       ACR_AGENT_PROFILE=claude   full recall benchmark      (-> benchmark/)
#   make check                                  validate problems.yaml     (-> benchmark/)
#   make test                                   run the test suites        (-> tests/)
#
# benchmark/smoke need ACR_AGENT_PROFILE=<name>;
# see benchmark/Makefile's header for the knobs (ACR_AGENT_PROFILE, PROBLEMS, KEEP).
# Per-suite test targets: see tests/Makefile.

.DEFAULT_GOAL := smoke

.PHONY: smoke benchmark check test

smoke:
	@$(MAKE) --no-print-directory -C benchmark smoke

benchmark:
	@$(MAKE) --no-print-directory -C benchmark benchmark

check:
	@$(MAKE) --no-print-directory -C benchmark check

test:
	@$(MAKE) --no-print-directory -C tests
