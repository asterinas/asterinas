#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Ensure the script is invoked with the necessary arguments
if [ "$#" -lt 2 ]; then
    echo "Atomic File Download with wget."
    echo "This script downloads a file from a specified URL using wget. \
        The downloaded file will appear locally as a complete file or not appear at all in case of an error."
    echo ""
    echo "Usage: $0 <output_file> <url> [wget_options]"
    echo ""
    echo "Arguments:"
    echo "  <output_file>   Name of the file to save the downloaded content."
    echo "  <url>           URL of the file to be downloaded."
    echo "  [wget_options]  Optional: Additional wget options for customized download behavior."
    exit 1
fi

OUTPUT_FILE="$1"
URL="$2"
WGET_OPTIONS="${3:-}"
TEMP_FILE="/tmp/$(basename "$OUTPUT_FILE")"

# Download the file using wget
if ! wget $WGET_OPTIONS -c -O "$TEMP_FILE" "$URL"; then
    echo "Error: Failed to download $URL." >&2
    rm -f "$TEMP_FILE"
    exit 1
fi

# Move the temporary file to the target location
if ! mv "$TEMP_FILE" "$OUTPUT_FILE"; then
    echo "Error: Failed to move $TEMP_FILE to $OUTPUT_FILE." >&2
    rm -f "$TEMP_FILE"
    exit 1
fi

echo "File downloaded successfully to $OUTPUT_FILE."
