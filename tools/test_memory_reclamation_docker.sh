#!/bin/bash
# SPDX-License-Identifier: MPL-2.0

# Test script for memory reclamation functionality using Docker

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Get the Docker image version
DOCKER_IMAGE_VERSION=$(cat DOCKER_IMAGE_VERSION)
DOCKER_IMAGE="asterinas-dev:${DOCKER_IMAGE_VERSION}"

echo -e "${YELLOW}Using Docker image: ${DOCKER_IMAGE}${NC}"

# Function to run tests in Docker
run_tests_in_docker() {
    local log_level=$1
    echo -e "\n${YELLOW}Running tests with RUST_LOG=${log_level}${NC}"
    
    docker run --rm \
        -v "$(pwd):/workspace" \
        -w /workspace \
        -e RUST_LOG="${log_level}" \
        "${DOCKER_IMAGE}" \
        bash -c "
            cargo test --package ostd --lib mm::frame::reclaimer::tests -- --nocapture
        "
}

# Run tests with different log levels
echo -e "${YELLOW}Starting memory reclamation tests...${NC}"

# Run with debug logging
run_tests_in_docker "debug"

# Check if tests passed
if [ $? -eq 0 ]; then
    echo -e "${GREEN}Debug level tests passed!${NC}"
else
    echo -e "${RED}Debug level tests failed!${NC}"
    exit 1
fi

# Run with info logging
run_tests_in_docker "info"

# Check if tests passed
if [ $? -eq 0 ]; then
    echo -e "${GREEN}Info level tests passed!${NC}"
else
    echo -e "${RED}Info level tests failed!${NC}"
    exit 1
fi

# Run with trace logging for detailed output
run_tests_in_docker "trace"

# Check if tests passed
if [ $? -eq 0 ]; then
    echo -e "${GREEN}Trace level tests passed!${NC}"
else
    echo -e "${RED}Trace level tests failed!${NC}"
    exit 1
fi

echo -e "\n${GREEN}All memory reclamation tests completed successfully!${NC}" 