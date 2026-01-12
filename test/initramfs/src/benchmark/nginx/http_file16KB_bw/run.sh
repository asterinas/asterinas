#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cp /benchmark/nginx/nginx.conf /benchmark/nginx/conf/
/benchmark/nginx/generate_random_html.sh 16384

mkdir -p /var/log/nginx

echo "Running nginx server"
/benchmark/bin/nginx -c /benchmark/nginx/conf/nginx.conf
