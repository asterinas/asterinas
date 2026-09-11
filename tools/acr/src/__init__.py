"""Aster Code Review runtime.

The package keeps orchestration and deterministic review mechanics independent
from the provider used to run model-backed agents.
"""

from .config import RunConfig, load_config

__all__ = ["RunConfig", "load_config"]
