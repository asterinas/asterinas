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
    parser.add_argument("--backend", choices=("openai-agents", "pi-agent", "fake"), default=None)
    parser.add_argument("--config", type=Path, default=None)
    args, passthrough = parser.parse_known_args(argv)
    # Interface flags such as --overwrite belong to the raw skill argument
    # grammar. argparse only owns the two process-level options above.
    raw_tokens = [*args.raw_args, *passthrough]
    raw = " ".join(f'"{token}"' if any(char.isspace() for char in token) else token for token in raw_tokens)
    try:
        config = load_config(args.config, overrides={"backend": args.backend} if args.backend else None)
        import asyncio
        asyncio.run(run_review(raw, config))
    except (ConfigError, RuntimeError, ValueError) as exc:
        print(f"acr: {exc}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
