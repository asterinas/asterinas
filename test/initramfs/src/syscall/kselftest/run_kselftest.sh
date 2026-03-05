#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

echo "=== Kselftest Runner Started ==="

KSELFTEST_DIR=$(dirname "$0")

# The `kselftest-list.txt` is a generated file produced by the kselftest build.
#
# Each line describes a single kselftest case in the form:
#     <test-dir>:<test-command>
#
# Example lines (see blocklists):
#   acct:acct_syscall
#   cgroup:test_stress.sh
#   exec:binfmt_script.py
TESTS="$KSELFTEST_DIR"/kselftest-list.txt
if [ ! -r "$TESTS" ] ; then
	echo "$0: Could not find list of tests to run ($TESTS)" >&2
	available=""
else
	available="$(cat "$TESTS")"
	echo "Found $(echo "$available" | grep -v '^$' | wc -l) available test cases"
fi

echo "Processing blocklists..."
blocklists=""
BLOCKLISTS_DIR="$(dirname $0)/blocklists"
for blocklist_file in "$BLOCKLISTS_DIR"/*; do
    while IFS= read -r line || [ -n "$line" ]; do
        line=$(echo "$line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
		case "$line" in
			"#"*)
				continue ;;
			*:*)
				dir=$(echo "$line" | cut -d: -f1)
                command=$(echo "$line" | cut -d: -f2)
				if [ "$command" = "*" ]; then
					matched_lines=$(echo "$available" | grep "^$dir:")
					blocklists="$blocklists $matched_lines"
				else
					blocklists="$blocklists $line"
				fi
                ;;
            *)
                echo "Warning: Invalid format in blocklist: $line" >&2
                continue ;;
		esac
	done < "$blocklist_file"
done
blocklists="$(echo "$blocklists" | tr ' ' '\n' | grep -v '^$' | sort | uniq)"
blocked_count=$(echo "$blocklists" | wc -l)
echo "Total blocklist entries processed: $blocked_count"

testcases="$(echo "$available" | grep -vxF "$blocklists" | grep -v '^$')"
run_count=$(echo "$testcases" | wc -l)
echo "Test cases to be executed: $run_count"
if [ $run_count -eq 0 ]; then
	echo "No tests to run after applying blocklists"
	exit 0
fi

echo "================================"

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'
total_tests=$run_count
passed_tests=0
failed_tests=0
failed_list=""

dirs=$(echo "$testcases" | cut -d: -f1 | sort | uniq)
for dir in $dirs ; do
	echo "Running tests in dir: $dir"
	commands=$(echo "$testcases" | grep "^$dir:" | cut -d: -f2)
	for command in $commands ; do
		echo "[ PROCESS  ]: $dir:$command"
		if [ -d "$KSELFTEST_DIR/$dir" ] && [ -x "$KSELFTEST_DIR/$dir/$command" ]; then
			(cd "$KSELFTEST_DIR/$dir" && ./"$command")
			exit_code=$?

			if [ $exit_code -eq 0 ]; then
				echo -e "[  ${GREEN}PASSED${NC}  ]: $dir:$command"
				passed_tests=$((passed_tests + 1))
			else
				echo -e "[  ${RED}FAILED${NC}  ]: $dir:$command (exit code: $exit_code)"
				failed_tests=$((failed_tests + 1))
				failed_list="$failed_list$dir:$command\n"
			fi
		else
			echo -e "[  ${RED}FAILED${NC}  ]: $dir:$command (executable not found)"
			failed_tests=$((failed_tests + 1))
			failed_list="$failed_list$dir:$command\n"
		fi
	done
done

echo ""
echo "=========== Summary ============"
echo -e "$GREEN$passed_tests$NC of $GREEN$total_tests$NC test cases passed."
if [ $failed_tests -gt 0 ]; then
	echo -e "The $RED$failed_tests$NC failed test cases are as follows:"
	echo -e "$failed_list"
	exit 1
else
	echo ""
	echo "All kselftest tests passed."
	exit 0
fi
