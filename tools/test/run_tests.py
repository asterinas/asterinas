#!/usr/bin/python3

# Use cargo metadata to get the manifest in json format.
def get_manifest():
    import json
    import subprocess
    manifest = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"]
    )
    return json.loads(manifest)

# Run the user mode tests for the crates and exit if any test fails.
def run_usermode_tests(crates):
    import os
    import subprocess
    for crate in crates:
        print("Running tests for", crate)
        result = subprocess.check_call(["cargo", "test", "--manifest-path", crate + "/Cargo.toml"])
        if result != 0:
            print("Test failed for", crate)
            os.exit(result)

# The member id returned by the cargo metadata command is
# `<package name> <package name> (path+file:///<absolute path to member>)`.
# We need a relative path as we specify them in `Cargo.toml`.
def member_id_to_crate_rel_path(member_id):
    import os
    annotation = member_id.split(" ")[2]
    abs_path = annotation \
        .replace("(", "") \
        .replace(")", "") \
        .replace("path+file://", "")
    return os.path.relpath(abs_path, os.getcwd())

def main():
    import os
    manifest = get_manifest()
    usermode_testables = manifest["metadata"]["usermode_testable"]
    ktest_testables = manifest["metadata"]["ktest_testable"]
    untestables = manifest["metadata"]["untestable"]
    # A sanity check to make sure we have registered all crates.
    all_members = sorted([member_id_to_crate_rel_path(p["id"]) for p in manifest["packages"]])
    test_members = sorted(usermode_testables + ktest_testables + untestables + ["."])
    if (all_members != test_members):
        print("Test members does not match all the workspace members in Cargo.toml. "
                "Please setup the testablity of all the crates in Cargo.toml correctly.")
        os._exit(1)

    run_usermode_tests(usermode_testables)

if __name__ == "__main__":
    main()
