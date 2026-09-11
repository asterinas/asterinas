#!/usr/bin/env python3
"""Command-line entry point for the ACR SDK implementation."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .config import ConfigError, load_config
from .core.orchestrator import run_review


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run Aster Code Review")
    parser.add_argument("raw_args", nargs="+", help="review interface tokens; output is last positional")
    parser.add_argument("--config", type=Path, default=None)
    parser.add_argument("--per-persona-context", choices=("auto", "yes", "no"), default=None)
    args, passthrough = parser.parse_known_args(argv)
    # Interface flags such as --overwrite belong to the raw review argument
    # grammar. argparse only owns the process-level options above, which
    # override the environment and the profile.
    raw_tokens = [*args.raw_args, *passthrough]
    raw = " ".join(f'"{token}"' if any(char.isspace() for char in token) else token for token in raw_tokens)
    try:
        config = load_config(args.config, overrides={"agent.per_persona_context": args.per_persona_context})
        import asyncio
        asyncio.run(run_review(raw, config))
    except (ConfigError, RuntimeError, ValueError) as exc:
        print(f"acr: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
