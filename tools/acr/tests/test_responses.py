from __future__ import annotations

import asyncio
import copy
import json
import os
import unittest
from unittest.mock import patch

from acr.agents.openai_agents import OpenAIAgentsBackend
from acr.agents.openai_agents.responses import normalize_responses_input
from acr.agents.protocol import AgentRequest
from acr.config import ProviderConfig, RunConfig
from acr.core.contracts import CommentsEnvelope


class ResponsesTests(unittest.TestCase):
    def test_unknown_metadata_is_removed_from_protocol_envelopes(self):
        expected = [{
            "type": "reasoning", "id": "reasoning-1", "status": "completed",
            "summary": [{"type": "summary_text", "text": "Investigation."}],
            "encrypted_content": "opaque reasoning state",
        }, {
            "type": "message", "id": "message-1", "role": "assistant",
            "status": "completed", "phase": "commentary",
            "content": [{"type": "output_text", "text": "Checking.", "annotations": [{
                "type": "url_citation", "url": "https://example.invalid", "title": "Reference",
                "start_index": 0, "end_index": 2,
            }]}],
        }]
        incoming = copy.deepcopy(expected)
        for item in incoming:
            item.update(quality="high", relay_metadata={"anything": True})
        incoming[0]["summary"][0]["vendor_extra"] = True
        incoming[1]["content"][0]["vendor_extra"] = True
        incoming[1]["content"][0]["annotations"][0]["vendor_extra"] = True
        original = copy.deepcopy(incoming)
        self.assertEqual(normalize_responses_input(incoming), expected)
        self.assertEqual(incoming, original)

    def test_tool_payloads_and_call_ids_are_preserved(self):
        payload = json.dumps({"quality": "business data", "provider_data": None, "type": "message"})
        expected = [{
            "type": "function_call", "name": "inspect_code", "call_id": "call-1", "arguments": payload,
        }, {
            "type": "function_call_output", "call_id": "call-1", "output": payload,
        }]
        incoming = [{**item, "relay_extra": "discard"} for item in expected]
        self.assertEqual(normalize_responses_input(incoming), expected)

    def test_core_fields_are_not_invented_or_coerced(self):
        items = [{
            "type": "function_call", "name": "inspect_code", "arguments": {"quality": "opaque"},
            "relay_extra": "discard",
        }]
        self.assertEqual(normalize_responses_input(items), [{
            "type": "function_call", "name": "inspect_code", "arguments": {"quality": "opaque"},
        }])
        unknown = [{"type": "unknown_protocol_item", "relay_extra": True}]
        self.assertEqual(normalize_responses_input(unknown), unknown)

    def test_metadata_does_not_break_a_second_model_request(self):
        from agents import function_tool
        from openai.types.responses import Response, ResponseFunctionToolCall, ResponseOutputMessage, ResponseOutputText, ResponseReasoningItem

        tool_values = []
        payload = json.dumps({"quality": "business data", "provider_data": None})

        @function_tool
        def inspect_code(quality: str) -> str:
            tool_values.append(quality)
            return payload

        replies = [Response.model_construct(
            id="response-1", status="completed", usage=None,
            output=[
                ResponseReasoningItem(
                    type="reasoning", id="reasoning-1", summary=[], encrypted_content="opaque state",
                    quality="high", relay_metadata=True,
                ),
                ResponseFunctionToolCall(
                    type="function_call", name="inspect_code", call_id="call-1",
                    arguments='{"quality": "business data"}', relay_metadata=True,
                ),
            ],
        ), Response.model_construct(
            id="response-2", status="completed", usage=None,
            output=[ResponseOutputMessage(
                id="message-1", type="message", role="assistant", status="completed",
                content=[ResponseOutputText(type="output_text", text='{"comments": []}', annotations=[])],
            )],
        )]
        requests = []

        async def respond(*, request_kwargs, transport_kwargs):
            requests.append(request_kwargs)
            return replies[len(requests) - 1]

        with patch.dict(os.environ, {"ACR_TEST_API_KEY": "test-only"}, clear=True):
            backend = OpenAIAgentsBackend(RunConfig(
                review_model="openai/test-model", tracing_enabled=False,
                provider=ProviderConfig(adapter="any-llm", api_key_env="ACR_TEST_API_KEY"),
            ))
            model, resource = backend._build_model("openai/test-model")
            with (
                patch.object(backend, "_build_model", return_value=(model, resource)),
                patch.object(backend, "_hosted_tools", return_value=[inspect_code]),
                patch.object(model, "_call_any_llm_responses", side_effect=respond),
            ):
                response = asyncio.run(backend.run(AgentRequest("persona", "Review the code.", "code", CommentsEnvelope)))

        self.assertEqual(response.value.model_dump(), {"comments": []})
        self.assertEqual(tool_values, ["business data"])
        self.assertEqual(len(requests), 2)
        history = requests[1]["input"]
        self.assertTrue(all("quality" not in item and "relay_metadata" not in item for item in history))
        reasoning = next(item for item in history if item.get("type") == "reasoning")
        self.assertEqual(reasoning["encrypted_content"], "opaque state")
        call = next(item for item in history if item.get("type") == "function_call")
        result = next(item for item in history if item.get("type") == "function_call_output")
        self.assertEqual(call["call_id"], result["call_id"])
        self.assertEqual(json.loads(call["arguments"]), {"quality": "business data"})
        self.assertEqual(result["output"], payload)
