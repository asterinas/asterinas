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

# Install Claude Code.
ENV CLAUDE_CODE_RELEASE=2.1.223
RUN curl -fsSL https://claude.ai/install.sh | bash -s -- ${CLAUDE_CODE_RELEASE} \
    && claude --version

# Install Codex.
ENV CODEX_RELEASE=0.147.0
ENV CODEX_NON_INTERACTIVE=yes
RUN curl -fsSL https://chatgpt.com/codex/install.sh | sh \
    && codex --version

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
