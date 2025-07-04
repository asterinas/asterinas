#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Generate a html file with random contents for the given length under `/usr/local/nginx/html`
# Usage: ./generate_random_html.sh <length>

LEN=$1

# Ensure LEN is numeric and reasonable
if ! [ "$LEN" -eq "$LEN" ] || [ "$LEN" -lt 120 ]; then 
    echo "Error: LEN must be a numeric value greater than or equal to 120"
    exit 1
fi

DIRNAME=/benchmark/nginx/html
mkdir -p $DIRNAME
FILENAME=${DIRNAME}/${LEN}bytes.html

rm -f ${FILENAME}

# Base HTML content
HEADER_CONTENT="<!DOCTYPE html>
<html>
<head>
<title>Sample Page</title>
</head>
<body>
<h1>Hello World!</h1><p>"

# Write initial content to the file
echo "$HEADER_CONTENT" > $FILENAME

# Calculate remaining length
HEADER_LENGTH=${#HEADER_CONTENT}  # Calculate this dynamically
FOOTER_CONTENT="</p>
</body>
</html>"
CONTENT_LENGTH=$((LEN - HEADER_LENGTH - ${#FOOTER_CONTENT}-2))

# Ensure the calculated CONTENT_LENGTH is positive
if [ "$CONTENT_LENGTH" -gt 0 ]; then
    i=0
    while [ "$i" -lt "$CONTENT_LENGTH" ]
    do
        echo -n "a" >> $FILENAME
        i=$((i + 1))
    done
fi

# Write the footer content
echo "$FOOTER_CONTENT" >> $FILENAME
