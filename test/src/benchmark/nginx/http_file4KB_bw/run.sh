#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cp /benchmark/nginx/nginx.conf /usr/local/nginx/conf/
/benchmark/nginx/generate_random_html.sh 4096

echo "Running nginx server"
/usr/local/nginx/sbin/nginx