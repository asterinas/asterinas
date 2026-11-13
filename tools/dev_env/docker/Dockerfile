# SPDX-License-Identifier: MPL-2.0

# This is the default development image, which provides AI-agent and GitHub CLI
# tooling for Asterinas agent skills.

ARG BASE_VERSION
FROM asterinas/kernel-dev:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

WORKDIR /root

# Install dependencies for the agent skills workflows and their self-tests.
RUN apt update \
    && apt install -y --no-install-recommends \
    gh \
    python3-yaml \
    && apt clean \
    && rm -rf /var/lib/apt/lists/*

# Install ACR's Python dependencies.
COPY tools/acr/pyproject.toml /tmp/acr-pyproject.toml
ENV ACR_PYTHON=/opt/acr-venv/bin/python
RUN <<'EOF'
set -e
python3 -m venv /opt/acr-venv
"${ACR_PYTHON}" - <<'PYTHON'
import subprocess
import sys
import tomllib

with open("/tmp/acr-pyproject.toml", "rb") as file:
    dependencies = tomllib.load(file)["project"]["dependencies"]
subprocess.run(
    [sys.executable, "-m", "pip", "install", "--no-cache-dir", *dependencies],
    check=True,
)
PYTHON
rm /tmp/acr-pyproject.toml
EOF

# Install Node.js and Pi.
FROM node:24.22.2-bookworm-slim AS pi-runtime
ENV PI_RELEASE=1.5.2
RUN npm install --global --ignore-scripts "@earendil-works/pi-coding-agent@${PI_RELEASE}"

FROM build-base
COPY --from=pi-runtime /usr/local/bin/node /usr/local/bin/node
COPY --from=pi-runtime /usr/local/lib/node_modules/@earendil-works/pi-coding-agent \
    /usr/local/lib/node_modules/@earendil-works/pi-coding-agent
RUN ln -s ../lib/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js /usr/local/bin/pi \
    && node --version \
    && pi --version

# Install Claude Code.
ENV CLAUDE_CODE_RELEASE=2.1.223
RUN curl -fsSL https://claude.ai/install.sh | bash -s -- ${CLAUDE_CODE_RELEASE} \
    && claude --version

# Install Codex.
ENV CODEX_RELEASE=0.160.0
ENV CODEX_NON_INTERACTIVE=yes
RUN curl -fsSL https://chatgpt.com/codex/install.sh | sh \
    && codex --version

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
