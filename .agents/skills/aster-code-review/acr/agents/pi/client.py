"""One-shot asynchronous client for the Node Pi SDK bridge."""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

from ...config import PiConfig
from ...tools.protocol import ToolBroker
from .protocol import PiProtocolError, validate_ready


MAX_FRAME_BYTES = 4 * 1024 * 1024


class PiBridgeError(RuntimeError):
    pass


class PiBridgeClient:
    def __init__(
        self,
        config: PiConfig,
        *,
        broker: ToolBroker | None,
        session_root: Path,
        event_hook: Callable[[dict[str, Any]], None] | None = None,
    ):
        self.config = config
        self.broker = broker
        self.session_root = session_root
        self.event_hook = event_hook
        self.process: asyncio.subprocess.Process | None = None
        self._stderr_task: asyncio.Task[None] | None = None

    async def run(self, frame: dict[str, Any]) -> dict[str, Any]:
        bridge = Path(__file__).resolve().parent / "runtime" / "bridge.mjs"
        env = os.environ.copy()
        env["PI_CODING_AGENT_DIR"] = str(self.config.agent_dir.resolve())
        env["ACR_PI_AGENT_DIR"] = str(self.config.agent_dir.resolve())
        native_session_dir = self.session_root / str(frame["id"])
        native_session_dir.mkdir(parents=True, mode=0o700, exist_ok=False)
        env["PI_CODING_AGENT_SESSION_DIR"] = str(native_session_dir)
        env["ACR_PI_TOOL_EXTENSION"] = str(
            bridge.with_name("tool_extension.ts").resolve()
        )
        started = time.monotonic()
        try:
            self.process = await asyncio.create_subprocess_exec(
                self.config.node_command,
                str(bridge),
                stdin=asyncio.subprocess.PIPE,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                env=env,
            )
        except OSError as exc:
            raise PiBridgeError(f"cannot start Pi bridge: {exc}") from exc
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        self._stderr_task = asyncio.create_task(self._drain_stderr())
        try:
            ready = await asyncio.wait_for(
                self._read_frame(),
                timeout=self.config.bridge_startup_timeout_seconds,
            )
            validate_ready(ready)
            guideline_script = Path(__file__).resolve().parents[2] / "scripts" / "print_guideline.py"
            ready["guideline_script"] = {
                "path": str(guideline_script),
                "sha256": hashlib.sha256(guideline_script.read_bytes()).hexdigest(),
            }
            ready["startup_duration_ms"] = int((time.monotonic() - started) * 1000)
            self._emit(ready)
            await self._write_frame(frame)
            while True:
                response = await self._read_frame()
                frame_type = response.get("type")
                if frame_type == "tool.call":
                    await self._handle_tool_call(response, frame["id"])
                    continue
                self._emit(response)
                if frame_type in {"result", "error"} and response.get("id") == frame["id"]:
                    return response
        except (asyncio.CancelledError, asyncio.TimeoutError):
            await self._cancel(frame["id"])
            raise
        except PiProtocolError:
            raise
        except Exception as exc:
            if isinstance(exc, PiBridgeError):
                raise
            raise PiBridgeError(f"Pi bridge protocol failed: {exc}") from exc
        finally:
            await self.close()

    async def close(self) -> None:
        process = self.process
        self.process = None
        if process is not None and process.returncode is None:
            try:
                await asyncio.wait_for(
                    process.wait(), timeout=self.config.shutdown_grace_seconds
                )
            except asyncio.TimeoutError:
                process.terminate()
                try:
                    await asyncio.wait_for(
                        process.wait(), timeout=self.config.shutdown_grace_seconds
                    )
                except asyncio.TimeoutError:
                    process.kill()
                    await process.wait()
        if self._stderr_task is not None:
            try:
                await asyncio.wait_for(self._stderr_task, timeout=1.0)
            except (asyncio.TimeoutError, asyncio.CancelledError):
                self._stderr_task.cancel()
            self._stderr_task = None

    async def _cancel(self, run_id: str) -> None:
        try:
            await self._write_frame(
                {"version": 1, "type": "cancel", "id": run_id}
            )
        except Exception:
            pass

    async def _handle_tool_call(self, frame: dict[str, Any], run_id: str) -> None:
        call_id = frame.get("call_id")
        tool = frame.get("tool")
        arguments = frame.get("arguments")
        if (
            frame.get("run_id") != run_id
            or not isinstance(call_id, str)
            or not isinstance(tool, str)
            or not isinstance(arguments, dict)
            or self.broker is None
        ):
            result = {"ok": False, "value": None, "error_code": "TOOL_DENIED"}
        else:
            try:
                invoked = await self.broker.invoke(tool, arguments)
                result = {
                    "ok": invoked.ok,
                    "value": invoked.value,
                    "error_code": invoked.error_code,
                }
            except Exception as exc:
                result = {
                    "ok": False,
                    "value": str(exc),
                    "error_code": type(exc).__name__.upper(),
                }
        await self._write_frame(
            {
                "version": 1,
                "type": "tool.result",
                "run_id": run_id,
                "call_id": call_id,
                **result,
            }
        )

    async def _read_frame(self) -> dict[str, Any]:
        assert self.process is not None and self.process.stdout is not None
        raw = await self.process.stdout.readline()
        if not raw:
            code = await self.process.wait()
            raise PiBridgeError(f"Pi bridge exited before a terminal frame (status {code})")
        if len(raw) > MAX_FRAME_BYTES or not raw.endswith(b"\n"):
            raise PiProtocolError("Pi bridge emitted an oversized or unterminated frame")
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise PiProtocolError("Pi bridge stdout is not valid JSONL") from exc
        if not isinstance(value, dict) or value.get("version") != 1:
            raise PiProtocolError("Pi bridge emitted an invalid protocol frame")
        return value

    async def _write_frame(self, value: dict[str, Any]) -> None:
        assert self.process is not None and self.process.stdin is not None
        encoded = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
        if len(encoded) > MAX_FRAME_BYTES:
            raise PiProtocolError("Pi bridge request frame exceeds 4 MiB")
        self.process.stdin.write(encoded + b"\n")
        await self.process.stdin.drain()

    async def _drain_stderr(self) -> None:
        assert self.process is not None and self.process.stderr is not None
        while raw := await self.process.stderr.readline():
            self._emit(
                {
                    "version": 1,
                    "type": "diagnostic",
                    "stream": "stderr",
                    "message": raw.decode("utf-8", "replace").rstrip()[:8192],
                }
            )

    def _emit(self, event: dict[str, Any]) -> None:
        if self.event_hook is not None:
            self.event_hook(event)
