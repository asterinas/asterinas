#! /bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

if type bazel > /dev/null; then
    echo "Bazel has been installed already"
    exit 0
fi

apt update && apt install curl gnupg -y
curl -fsSL https://bazel.build/bazel-release.pub.gpg | gpg --dearmor > bazel.gpg
mv bazel.gpg /etc/apt/trusted.gpg.d/

echo 'deb [arch=amd64] https://storage.googleapis.com/bazel-apt stable jdk1.8' | tee /etc/apt/sources.list.d/bazel.list
apt update && apt install bazel=5.4.0 -y

echo "Bazel is installed successfully"
