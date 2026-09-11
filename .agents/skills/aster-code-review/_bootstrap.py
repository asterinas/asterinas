#!/usr/bin/env python3
"""Load the relocated ACR package under its existing import name."""

from __future__ import annotations

import importlib.util
import runpy
import sys
from pathlib import Path


if __name__ == "__main__":
    root = Path(__file__).resolve().parent
    spec = importlib.util.spec_from_file_location(
        "acr", root / "__init__.py", submodule_search_locations=[str(root)]
    )
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load ACR package from {root}")
    package = importlib.util.module_from_spec(spec)
    sys.modules["acr"] = package
    spec.loader.exec_module(package)

    module = sys.argv.pop(1)
    runpy.run_module(module, run_name="__main__", alter_sys=True)
