"""Own one SDK session and shield CLI cleanup from asyncio cancellation."""

from __future__ import annotations

import asyncio
import math
from contextlib import suppress
from typing import Any, Awaitable, Callable

import anyio

from ..protocol import CancellationToken


class ClaudeSession:
    def __init__(self, client: Any, cancellation: CancellationToken):
        self.client = client
        self.cancellation = cancellation
        self._owner: asyncio.Task | None = None
        self._scope: anyio.CancelScope | None = None
        self._stopping = False

    async def run(self, operation: Callable[[], Awaitable[Any]], timeout: float | None) -> Any:
        # asyncio.wait_for/task.cancel must never interrupt SDK disconnect().
        # Only the owner enters SDK scopes; external cancellation signals its
        # AnyIO scope, then waits for that owner to finish reaping the CLI.
        self._owner = asyncio.create_task(self._run(operation, timeout))
        try:
            return await asyncio.shield(self._owner)
        except asyncio.CancelledError:
            self.stop()
            await self.wait_closed()
            raise

    async def _run(self, operation: Callable[[], Awaitable[Any]], timeout: float | None) -> Any:
        connected = False
        deadline = anyio.current_time() + timeout if timeout is not None else math.inf
        with anyio.CancelScope(deadline=deadline) as scope:
            self._scope = scope
            watcher = asyncio.create_task(self._watch_cancellation())
            try:
                if self._stopping or self.cancellation.cancelled:
                    scope.cancel()
                await self.client.connect()
                connected = True
                value = await operation()
            finally:
                scope.deadline = math.inf
                with anyio.CancelScope(shield=True):
                    if connected and scope.cancel_called:
                        with anyio.move_on_after(1), suppress(Exception):
                            await self.client.interrupt()
                    try:
                        await self.client.disconnect()
                    finally:
                        watcher.cancel()
                        with suppress(asyncio.CancelledError):
                            await watcher
                        self._scope = None
        if scope.cancel_called:
            if self._stopping or self.cancellation.cancelled:
                raise asyncio.CancelledError
            raise TimeoutError("Claude agent timed out")
        return value

    async def _watch_cancellation(self) -> None:
        await self.cancellation.wait()
        self.stop()

    def stop(self) -> None:
        self._stopping = True
        if self._scope is not None:
            self._scope.cancel()

    async def wait_closed(self) -> None:
        if self._owner is None:
            return
        with anyio.CancelScope(shield=True):
            while not self._owner.done():
                try:
                    await asyncio.shield(self._owner)
                except asyncio.CancelledError:
                    continue
                except Exception:
                    break
        # Retrieve terminal exceptions even when caller cancellation won the race.
        with suppress(asyncio.CancelledError, Exception):
            self._owner.result()
