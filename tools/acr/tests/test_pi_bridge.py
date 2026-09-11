"""Exercise bridge shutdown with a deterministic SDK and a slow pipe reader."""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import tempfile
import textwrap
import unittest
from pathlib import Path


_SDK = """
export const ModelRuntime = {
    create: async () => ({ hasConfiguredAuth: () => true }),
};
export const resolveCliModel = () => ({ model: { provider: "fixture" } });
export class DefaultResourceLoader {
    async reload() {}
    getExtensions() { return { errors: [] }; }
}
export const SessionManager = { inMemory() {} };
export async function createAgentSession({ customTools }) {
    let listener;
    return { session: {
        messages: [],
        extensionRunner: { hasHandlers: () => false },
        async bindExtensions() {},
        getActiveToolNames: () => customTools.map(tool => tool.name),
        subscribe(callback) { listener = callback; return () => {}; },
        async prompt(mode) {
            // Every frame fits the limit, but their total fills the stdout pipe.
            for (let index = 0; index < 32; index++) {
                listener({ type: "message_update", index, text: "x".repeat(256 * 1024) });
            }
            if (mode === "error") throw new Error("fixture failure");
            await customTools.find(tool => tool.name === "structured_output")
                .execute("fixture-output", { ok: true });
        },
        dispose() { process.stderr.write("session disposed\\n"); },
    } };
}
"""


@unittest.skipUnless(shutil.which("node"), "Node is an optional test dependency")
class PiBridgeShutdownTests(unittest.IsolatedAsyncioTestCase):
    async def test_slow_reader_receives_all_progress_and_terminal_frames(self):
        with tempfile.TemporaryDirectory(prefix="acr-pi-bridge-") as directory:
            root = Path(directory)
            (root / "dist").mkdir()
            (root / "package.json").write_text(
                json.dumps({"type": "module", "version": "fixture"}), encoding="utf-8",
            )
            (root / "dist" / "index.js").write_text(textwrap.dedent(_SDK), encoding="utf-8")
            bridge = Path(__file__).resolve().parents[1] / "src/agents/pi/runtime/bridge.mjs"
            env = {**os.environ, "ACR_PI_SDK_ROOT": str(root), "ACR_PI_AGENT_DIR": str(root)}
            for mode in ("result", "error"):
                with self.subTest(terminal=mode):
                    process = await asyncio.create_subprocess_exec(
                        "node", str(bridge), env=env,
                        stdin=asyncio.subprocess.PIPE,
                        stdout=asyncio.subprocess.PIPE,
                        stderr=asyncio.subprocess.PIPE,
                    )
                    try:
                        ready = await asyncio.wait_for(process.stdout.readline(), timeout=5)
                        self.assertEqual(json.loads(ready)["type"], "ready")
                        request = {
                            "version": 1, "type": "run", "id": "fixture-run",
                            "request": {
                                "cwd": str(root), "model": "fixture/mock",
                                "system_prompt": "Fixture instructions.", "user_prompt": mode,
                                "output_schema": {"type": "object"},
                            },
                        }
                        process.stdin.write(json.dumps(request).encode() + b"\n")
                        await process.stdin.drain()
                        disposed = await asyncio.wait_for(process.stderr.readline(), timeout=5)
                        self.assertEqual(disposed, b"session disposed\n")
                        # Keep stdout blocked beyond the former 10 ms exit timer.
                        await asyncio.sleep(0.1)
                        stdout, stderr = await asyncio.wait_for(process.communicate(), timeout=5)
                        self.assertEqual(process.returncode, 0, stderr.decode())
                        lines = stdout.splitlines(keepends=True)
                        self.assertTrue(all(line.endswith(b"\n") for line in lines), "truncated JSONL frame")
                        frames = [json.loads(line) for line in lines]
                        progress = [frame for frame in frames if frame["type"] == "progress"]
                        self.assertEqual([frame["event"]["index"] for frame in progress], list(range(32)))
                        terminals = [frame for frame in frames if frame["type"] in {"result", "error"}]
                        self.assertEqual(len(terminals), 1)
                        self.assertEqual(frames[-1], terminals[0])
                        self.assertEqual(terminals[0]["type"], mode)
                        if mode == "result":
                            self.assertEqual(terminals[0]["value"], {"ok": True})
                        else:
                            self.assertEqual(terminals[0]["error"], "fixture failure")
                    finally:
                        if process.returncode is None:
                            process.kill()
                        await process.communicate()
