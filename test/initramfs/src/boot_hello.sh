#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

die() {
  echo "Error: $*" >&2
  exit 1
}

print_args() {
  echo "Arguments ($#):"
  if [ "$#" -eq 0 ]; then
    echo "  (none)"
    return 0
  fi

  idx=0
  for arg in "$@"; do
    idx=$((idx + 1))
    printf '  [%d] %s\n' "$idx" "$arg"
  done
}

print_env_sorted() {
  echo "Environment variables:"
  env | sort | sed 's/^/  /'
}

arg_contains_exact() {
  needle="$1"
  shift
  printf '%s\n' "$@" | grep -x -q -- "$needle"
}

env_contains_exact() {
  needle="$1"
  env | grep -x -q -- "$needle"
}

env_contains_prefix() {
  prefix="$1"
  env | grep -q -- "^${prefix}"
}

assert_arg_contains_exact() {
  needle="$1"
  shift
  arg_contains_exact "$needle" "$@" || die "\`$needle\` should be forwarded to \`init\` as argument"
}

refute_arg_contains_exact() {
  needle="$1"
  shift
  if arg_contains_exact "$needle" "$@"; then
    die "\`$needle\` should be ignored and not forwarded to \`init\`"
  fi
}

assert_env_contains_exact() {
  needle="$1"
  env_contains_exact "$needle" || die "\`$needle\` should be forwarded to \`init\` as env"
}

refute_env_contains_exact() {
  needle="$1"
  if env_contains_exact "$needle"; then
    die "\`$needle\` should be ignored and not forwarded to \`init\`"
  fi
}

refute_env_contains_prefix() {
  prefix="$1"
  if env_contains_prefix "$prefix"; then
    die "\`${prefix}\` should be consumed by kernel"
  fi
}

print_args "$@"
print_env_sorted

# Verify forwarding/ignoring behavior.
assert_arg_contains_exact 'undefined-flag' "$@"
refute_arg_contains_exact 'undefined-key' "$@"
refute_arg_contains_exact 'undefined_module.flag' "$@"

refute_env_contains_exact 'undefined_key=1'
assert_env_contains_exact 'undefined-key=2'
refute_env_contains_exact 'undefined_module.key=val'

refute_env_contains_prefix 'init='
refute_env_contains_prefix 'console='

echo "Successfully booted."
