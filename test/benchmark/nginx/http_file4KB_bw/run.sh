#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cp /benchmark/nginx/nginx.conf /benchmark/nginx/conf/
/benchmark/nginx/generate_random_html.sh 4096

echo "Running nginx server"
/benchmark/bin/nginx -e /benchmark/nginx/error.log -c /benchmark/nginx/conf/nginx.conf
