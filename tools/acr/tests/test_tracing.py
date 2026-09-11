from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path

from acr.agents.openai_agents import sdk_trace_context
from acr.config import RunConfig
from acr.core.events import EventLogger


class TracingTests(unittest.TestCase):
    def test_response_span_includes_full_input_and_response(self):
        from agents import response_span

        class FakeResponse:
            id = "resp_test"

            @staticmethod
            def model_dump(*, mode="python"):
                return {
                    "id": "resp_test",
                    "output": [{"type": "message", "text": "final answer"}],
                    "mode": mode,
                }

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            logger = EventLogger(root, "trace-test")
            config = RunConfig(backend="openai-agents")
            with sdk_trace_context(logger, config):
                with response_span() as span:
                    span.span_data.input = [{"role": "user", "content": "review this"}]
                    span.span_data.response = FakeResponse()

            events = [json.loads(line) for line in (root / "sdk-trace.jsonl").read_text().splitlines()]
            completed = next(
                event
                for event in events
                if event["event_type"] == "sdk.span.completed"
                and event["span"]["span_data"]["type"] == "response"
            )
            data = completed["span"]["span_data"]
            self.assertEqual(data["input"][0]["content"], "review this")
            self.assertEqual(data["response"]["output"][0]["text"], "final answer")

    def test_sdk_trace_writes_workflow_and_scoped_span_content(self):
        from agents import custom_span

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            logger = EventLogger(root, "trace-test", max_content_bytes=1024)
            config = RunConfig(backend="openai-agents", trace_max_content_bytes=1024)
            with sdk_trace_context(logger, config):
                with custom_span(
                    "acr.agent.security",
                    {
                        "acr_stage": "persona",
                        "acr_persona": "security",
                        "acr_agent_run_id": "agent-1",
                        "input": "read kernel/src/lib.rs",
                    },
                ):
                    pass

            events = [json.loads(line) for line in (root / "sdk-trace.jsonl").read_text().splitlines()]
            self.assertEqual(events[0]["event_type"], "sdk.trace.started")
            completed = [event for event in events if event["event_type"] == "sdk.span.completed"]
            self.assertEqual(len(completed), 1)
            self.assertEqual(completed[0]["persona"], "security")
            self.assertEqual(completed[0]["agent_run_id"], "agent-1")
            self.assertIn("read kernel/src/lib.rs", json.dumps(completed[0], ensure_ascii=False))
            self.assertTrue((root / "agents/personas/security/sdk-trace.jsonl").exists())

    def test_sdk_trace_redacts_provider_specific_key_in_payload(self):
        from agents import custom_span

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            old = os.environ.get("TOKENSKINGDOM_API_KEY")
            os.environ["TOKENSKINGDOM_API_KEY"] = "provider-secret-value"
            try:
                logger = EventLogger(root, "trace-test")
                config = RunConfig(backend="openai-agents")
                with sdk_trace_context(logger, config):
                    with custom_span("secret", {"payload": "provider-secret-value"}):
                        pass
            finally:
                if old is None:
                    os.environ.pop("TOKENSKINGDOM_API_KEY", None)
                else:
                    os.environ["TOKENSKINGDOM_API_KEY"] = old
            text = (root / "sdk-trace.jsonl").read_text()
            self.assertNotIn("provider-secret-value", text)
            self.assertIn("[REDACTED]", text)


if __name__ == "__main__":
    unittest.main()
