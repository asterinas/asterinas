import re
import os

def analyze_and_generate_whitelist(log_filename="xfstests.log", whitelist_filename="white.list"):
    """
    Parses xfstests.log, identifies passed tests, and generates a whitelist file.
    """
    if not os.path.exists(log_filename):
        print(f"Error: Log file '{log_filename}' not found.")
        return

    passed_tests = []
    failed_count = 0
    not_run_count = 0

    # Regex Pattern: 
    # Group 1: Test name (e.g., generic/001)
    # Group 2: Status information
    test_line_re = re.compile(r'^([\w-]+/\d+)\s+(.*)')

    try:
        with open(log_filename, 'r', encoding='utf-8') as f:
            for line in f:
                line = line.strip()
                match = test_line_re.match(line)
                
                if match:
                    test_name = match.group(1)
                    status_info = match.group(2)

                    # Classification Logic
                    if "[not run]" in status_info or "[expunged]" in status_info:
                        not_run_count += 1
                    elif "[failed" in status_info:
                        failed_count += 1
                    # A test is 'Passed' if it shows runtime (e.g., 2s) and has no error brackets
                    elif re.search(r'\d+s', status_info) and "[" not in status_info:
                        passed_tests.append(test_name)
        
        # Write passed tests to whitelist file
        if passed_tests:
            with open(whitelist_filename, 'w', encoding='utf-8') as wf:
                for test in passed_tests:
                    wf.write(f"{test}\n")
            
        # Print Summary Report
        print("=" * 60)
        print(f"{'xfstests Result Analysis & Whitelist Generation':^60}")
        print("=" * 60)
        print(f"Total Passed:      {len(passed_tests):>4}")
        print(f"Total Failed:      {failed_count:>4}")
        print(f"Total Not Run:     {not_run_count:>4}")
        print("-" * 60)
        
        if passed_tests:
            print(f"SUCCESS: Whitelist generated at '{whitelist_filename}'")
            print(f"You can now run: ./run-xfstests.sh $(cat {whitelist_filename})")
            print("-" * 60)
            print("\n[PASSED TESTS INCLUDED IN WHITELIST]:")
            for i in range(0, len(passed_tests), 4):
                print("  " + "  ".join(f"{t:<14}" for t in passed_tests[i:i+4]))
        else:
            print("WARNING: No passed tests found. Whitelist was not created.")

    except Exception as e:
        print(f"An error occurred: {e}")

if __name__ == "__main__":
    analyze_and_generate_whitelist()