"""Model-free tests for ACR.

The suite must behave the same on every machine, so it never inherits the
caller's ACR settings. Only the location of an optional Pi SDK, which some
tests use when present, is kept.
"""

import os

for _name in [name for name in os.environ if name.startswith("ACR_")]:
    if _name != "ACR_PI_SDK_ROOT":
        del os.environ[_name]
