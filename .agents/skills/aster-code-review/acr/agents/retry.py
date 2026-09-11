"""Whole-agent retry support shared by post-processing stages."""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from typing import TypeVar


T = TypeVar("T")


async def run_with_retries(operation: Callable[[], Awaitable[T]], retries: int) -> T:
    """Retry a complete agent operation, including its output validation."""

    error: Exception | None = None
    for attempt in range(retries + 1):
        try:
            return await operation()
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            error = exc
            if attempt < retries:
                await asyncio.sleep(min(2.0, 0.1 * (2**attempt)))
    assert error is not None
    raise error
