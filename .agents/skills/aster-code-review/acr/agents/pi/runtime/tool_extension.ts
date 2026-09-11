import { Type } from "@sinclair/typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Registers ACR's allowlisted tools in the Pi child runtime.

type AcrToolSpec = {
	name: string;
	sdk_name: string;
	description: string;
	input_schema: Record<string, unknown>;
};

type AcrToolRuntime = {
	runId: string;
	tools: AcrToolSpec[];
	invoke(tool: string, argumentsValue: Record<string, unknown>, signal?: AbortSignal): Promise<{
		ok: boolean;
		value?: unknown;
		error_code?: string | null;
	}>;
	acknowledge(): void;
};

const runtime = (globalThis as Record<PropertyKey, unknown>)[
	Symbol.for("acr.pi.tool-runtime.v1")
] as AcrToolRuntime | undefined;

if (!runtime) throw new Error("ACR Pi tool runtime is unavailable");

export default function acrToolExtension(pi: ExtensionAPI): void {
	for (const spec of runtime.tools) {
		pi.registerTool({
			name: spec.sdk_name,
			label: spec.name,
			description: spec.description,
			parameters: Type.Unsafe(spec.input_schema),
			executionMode: "parallel",
			async execute(_toolCallId, params, signal) {
				const result = await runtime.invoke(
					spec.name,
					params as Record<string, unknown>,
					signal,
				);
				if (!result.ok) {
					throw new Error(`${result.error_code ?? "TOOL_ERROR"}: ${String(result.value ?? "tool failed")}`);
				}
				const text = typeof result.value === "string"
					? result.value
					: JSON.stringify(result.value);
				return { content: [{ type: "text", text }], details: { canonicalName: spec.name } };
			},
		});
	}
	pi.on("session_start", () => runtime.acknowledge());
}
