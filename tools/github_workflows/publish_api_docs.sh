#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# This script is used to build API documentations 
# and upload the documentation to self-hosted repos.

# Print help message
print_help() {
    echo "Usage: $0 [--dry-run | --nightly <key_file> | --release <key_file>]"
    echo ""
    echo "Options:"
    echo "    --dry-run    Build documentation without uploading."
    echo "    nightly      Build and upload nightly API documentation."
    echo "    release      Build and upload API documentation for a new version."
    echo "    key_file     Path to the file that stores the SSH key. Required if documentation needs to be uploaded."
}

# Validate the command line parameters
validate_parameter() {
    if [ "$1" = "--dry-run" ]; then
        if [ "$#" -ne 1 ]; then
            echo "Error: '--dry-run' mode does not accept additional arguments."
            print_help
            exit 1
        fi
        return
    fi

    if [ "$1" != "nightly" ] && [ "$1" != "release" ]; then
        echo "Error: Invalid option. Please provide either 'nightly' or 'release' as the first parameter."
        print_help
        exit 1
    fi

    if [ "$#" -ne 2 ]; then
        echo "Error: Please provide both the option and file parameters."
        print_help
        exit 1
    fi

    if [ ! -f "$2" ]; then
        echo "Error: File not found. Please provide a valid file path as the second parameter."
        print_help
        exit 1
    fi
}

# Build documentation of all crates to publish
build_api_docs() {
    cd "${ASTER_SRC_DIR}"
    make install_osdk
    # Crates depend on OSTD, including OSTD itself
    CRATES="ostd osdk/deps/frame-allocator osdk/deps/heap-allocator osdk/deps/test-kernel"
    for CRATE in $CRATES; do
        pushd $CRATE
        RUSTDOCFLAGS="-Dwarnings" cargo osdk doc
        popd
    done
}

# Git clone the API documentation repo
clone_repo() {
    cd "${WORK_DIR}"
    chmod 600 "${SSH_KEY_FILE}"
    ssh-keygen -y -f "${SSH_KEY_FILE}" > /dev/null
    ssh-keyscan -t rsa github.com >> "${KNOWN_HOSTS_FILE}"
    git config --global user.email "github-actions[bot]@users.noreply.github.com"
    git config --global user.name "github-actions[bot]"
    GIT_SSH_COMMAND="ssh -i ${SSH_KEY_FILE} -o UserKnownHostsFile=${KNOWN_HOSTS_FILE}" git clone "${REPO_URL}" "${CLONED_REPO_DIR}"
}

# Generate the index.html for redirecting
generate_redirect_index_html() {
    local URL="$1"
    TEMPLATE="
<!DOCTYPE html>
<html>
<head>
    <meta http-equiv=\"refresh\" content=\"0; URL=${URL}\">
</head>
<body>
    <p>Redirecting to a new page...</p>
    <script>
        // If the browser doesn't support automatic redirection, display a link for manual redirection
        window.location.href = \"${URL}\";
    </script>
</body>
</html>
"
    echo -e "${TEMPLATE}" > index.html
}

# Update the nightly documentation and upload
update_nightly_doc() {
    cd "${WORK_DIR}/${CLONED_REPO_DIR}"
    git checkout --orphan new_branch
    rm -rf *
    cp -r ${ASTER_SRC_DIR}/target/x86_64-unknown-none/doc/* ./
    generate_redirect_index_html "https://asterinas.github.io/api-docs-nightly/ostd"
    git add .
    git commit -am "Update nightly API docs"
    git branch -D main
    git branch -m main
    GIT_SSH_COMMAND="ssh -i ${SSH_KEY_FILE} -o UserKnownHostsFile=${KNOWN_HOSTS_FILE}" git push -f origin main
    cd "${WORK_DIR}" && rm -rf "${WORK_DIR}/${CLONED_REPO_DIR}"
}

# Update the release documentation and upload
update_release_doc() {
    cd "${WORK_DIR}/${CLONED_REPO_DIR}"
    VERSION=$(cat "${ASTER_SRC_DIR}/VERSION")
    git rm -rf --ignore-unmatch "${VERSION}"
    mkdir "${VERSION}"
    cp -r ${ASTER_SRC_DIR}/target/x86_64-unknown-none/doc/* ${VERSION}/
    generate_redirect_index_html "https://asterinas.github.io/api-docs/${VERSION}/ostd"
    git add .
    git commit -am "Update API docs to v${VERSION}"
    GIT_SSH_COMMAND="ssh -i ${SSH_KEY_FILE} -o UserKnownHostsFile=${KNOWN_HOSTS_FILE}" git push -f origin main
    cd "${WORK_DIR}" && rm -rf "${WORK_DIR}/${CLONED_REPO_DIR}"
}

# Check if help message should be printed
if [ "$1" = "-h" ] || [ "$1" = "--help" ]; then
    print_help
    exit 0
fi

# Validate and retrieve script parameters
validate_parameter "$@"
SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/../..
WORK_DIR=${ASTER_SRC_DIR}/..
CLONED_REPO_DIR=temp_api_docs
KNOWN_HOSTS_FILE="${WORK_DIR}/known_hosts"

build_api_docs

if [ "$1" = "--dry-run" ]; then
    exit 0
fi

SSH_KEY_FILE=$(realpath "$2")
if [ "$1" = "nightly" ]; then
    REPO_URL=git@github.com:asterinas/api-docs-nightly.git
    clone_repo
    update_nightly_doc
elif [ "$1" = "release" ]; then
    REPO_URL=git@github.com:asterinas/api-docs.git
    clone_repo
    update_release_doc
else
    echo "Error: Invalid option. Please provide either 'nightly' or 'release' as the first parameter."
    print_help
    exit 1
fi
