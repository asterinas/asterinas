import re
import os

def analyze_and_generate_whitelist(log_filename="xfstests.log", whitelist_filename="white.list"):
    """
    Parses xfstests.log, identifies passed tests, and generates a whitelist file.

    Expected log format:
    - Ran: <list of all tests that were executed>
    - Not run: <list of tests that were skipped>
    - Failures: <list of tests that failed>
    - Failed X of Y tests

    Passed tests = Ran - Failures
    """
    if not os.path.exists(log_filename):
        print(f"Error: Log file '{log_filename}' not found.")
        return

    ran_tests = set()
    not_run_tests = set()
    failed_tests = set()

    # Regex to match test names like generic/001, ext4/042, shared/002
    test_name_re = re.compile(r'\b([\w-]+/\d+)\b')

    try:
        with open(log_filename, 'r', encoding='utf-8') as f:
            content = f.read()

            # Parse "Ran:" section
            ran_match = re.search(r'^Ran:\s*(.+?)(?=^Not run:|^Failures:|$)', content, re.MULTILINE | re.DOTALL)
            if ran_match:
                ran_tests = set(test_name_re.findall(ran_match.group(1)))

            # Parse "Not run:" section
            not_run_match = re.search(r'^Not run:\s*(.+?)(?=^Failures:|^Failed \d+|$)', content, re.MULTILINE | re.DOTALL)
            if not_run_match:
                not_run_tests = set(test_name_re.findall(not_run_match.group(1)))

            # Parse "Failures:" section
            failures_match = re.search(r'^Failures:\s*(.+?)(?=^Failed \d+|$)', content, re.MULTILINE | re.DOTALL)
            if failures_match:
                failed_tests = set(test_name_re.findall(failures_match.group(1)))

        # Calculate passed tests: Ran - Not Run - Failures
        # First, get tests that actually ran (excluding "Not run")
        actually_ran = ran_tests - not_run_tests
        # Then, get passed tests (actually ran - failures)
        passed_tests = sorted(actually_ran - failed_tests)

        # Calculate totals
        total_listed_in_ran = len(ran_tests)
        total_actually_ran = len(actually_ran)
        total_failed = len(failed_tests)
        total_not_run = len(not_run_tests)
        total_passed = len(passed_tests)

        # Verify consistency
        if total_passed + total_failed != total_actually_ran:
            print(f"WARNING: Inconsistent counts detected!")
            print(f"  Actually Ran: {total_actually_ran}, Failed: {total_failed}, Passed: {total_passed}")
            print(f"  Expected: Passed + Failed = Actually Ran")

        # Write passed tests to whitelist file
        if passed_tests:
            with open(whitelist_filename, 'w', encoding='utf-8') as wf:
                for test in passed_tests:
                    wf.write(f"{test}\n")

        # Print Summary Report
        print("=" * 70)
        print(f"{'xfstests Result Analysis & Whitelist Generation':^70}")
        print("=" * 70)
        print(f"Total Actually Ran: {total_actually_ran:>5}")
        print(f"Total Passed:       {total_passed:>5}")
        print(f"Total Failed:       {total_failed:>5}")
        print(f"Total Not Run:      {total_not_run:>5}")
        print("-" * 70)

        if passed_tests:
            print(f"SUCCESS: Whitelist generated at '{whitelist_filename}'")
            print(f"You can now run: ./run_xfstests.sh $(cat {whitelist_filename})")
            print("-" * 70)
            print("\n[PASSED TESTS INCLUDED IN WHITELIST]:")
            for i in range(0, len(passed_tests), 5):
                print("  " + "  ".join(f"{t:<15}" for t in passed_tests[i:i+5]))
        else:
            print("WARNING: No passed tests found. Whitelist was not created.")

        # Print failed tests if any
        if failed_tests:
            print("\n[FAILED TESTS]:")
            failed_list = sorted(failed_tests)
            for i in range(0, len(failed_list), 5):
                print("  " + "  ".join(f"{t:<15}" for t in failed_list[i:i+5]))

    except Exception as e:
        print(f"An error occurred: {e}")

if __name__ == "__main__":
    analyze_and_generate_whitelist()