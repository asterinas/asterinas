#!/usr/bin/env node

// ACR's one-session Pi Agent SDK subprocess bridge.

import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";
import { pathToFileURL } from "node:url";
import { randomUUID } from "node:crypto";

const protocolWrite = process.stdout.write.bind(process.stdout);
const diagnostic = (...values) => process.stderr.write(`${values.map(String).join(" ")}\n`);
process.stdout.write = (chunk, encoding, callback) => process.stderr.write(chunk, encoding, callback);
console.log = diagnostic;
console.info = diagnostic;
console.debug = diagnostic;
console.warn = diagnostic;
console.error = diagnostic;

const VERSION = 1;
const MAX_FRAME_BYTES = 4 * 1024 * 1024;
const OUTPUT_TOOL = "structured_output";
const DENIED_TOOLS = ["read", "grep", "find", "ls", "bash", "edit", "write", "powershell", "subagent", "contact_supervisor", "intercom"];

let terminalSent = false;

function emit(value) {
	const terminal = value.type === "result" || value.type === "error";
	if (terminal && terminalSent) {
		diagnostic("Pi bridge cleanup:", value.error ?? value.status);
		return;
	}
	const line = JSON.stringify({ version: VERSION, source_timestamp: new Date().toISOString(), ...value });
	if (Buffer.byteLength(line, "utf8") > MAX_FRAME_BYTES) throw new Error("output frame exceeds 4 MiB");
	protocolWrite(`${line}\n`);
	if (terminal) terminalSent = true;
}

function packageRootFromPi() {
	const explicit = process.env.ACR_PI_SDK_ROOT;
	if (explicit) return path.resolve(explicit);
	for (const directory of (process.env.PATH ?? "").split(path.delimiter)) {
		const candidate = path.join(directory, process.platform === "win32" ? "pi.cmd" : "pi");
		if (!fs.existsSync(candidate)) continue;
		let current = path.dirname(fs.realpathSync(candidate));
		while (current !== path.dirname(current)) {
			const manifest = path.join(current, "package.json");
			if (fs.existsSync(manifest)) {
				const parsed = JSON.parse(fs.readFileSync(manifest, "utf8"));
				if (parsed.name === "@earendil-works/pi-coding-agent") return current;
			}
			current = path.dirname(current);
		}
	}
	throw new Error("cannot locate the @earendil-works/pi-coding-agent installation");
}

const sdkRoot = packageRootFromPi();
const sdkManifest = JSON.parse(fs.readFileSync(path.join(sdkRoot, "package.json"), "utf8"));
const sdk = await import(pathToFileURL(path.join(sdkRoot, "dist", "index.js")).href);
const agentDir = path.resolve(process.env.ACR_PI_AGENT_DIR ?? path.join(process.env.HOME ?? "", ".pi", "agent"));

emit({
	type: "ready",
	bridge_version: "2",
	pi_version: sdkManifest.version,
	capabilities: ["agent_session", "structured_output", "usage", "tool_proxy", "cancellation"],
});

let activeRun;
const pendingTools = new Map();

function settleTool(frame) {
	const pending = pendingTools.get(frame.call_id);
	if (!pending || frame.run_id !== activeRun?.id) throw new Error("unknown tool result identity");
	pendingTools.delete(frame.call_id);
	pending.resolve({ ok: frame.ok === true, value: frame.value, error_code: frame.error_code });
}

function invokePythonTool(tool, argumentsValue, signal) {
	const callId = randomUUID();
	return new Promise((resolve, reject) => {
		const abort = () => {
			pendingTools.delete(callId);
			reject(new Error("tool call cancelled"));
		};
		if (signal?.aborted) return abort();
		signal?.addEventListener("abort", abort, { once: true });
		pendingTools.set(callId, {
			resolve(value) {
				signal?.removeEventListener("abort", abort);
				resolve(value);
			},
		});
		emit({ type: "tool.call", run_id: activeRun.id, call_id: callId, tool, arguments: argumentsValue });
	});
}

async function shutdownSession(session) {
	if (!session) return;
	try {
		if (session.extensionRunner.hasHandlers("session_shutdown")) {
			await session.extensionRunner.emit({ type: "session_shutdown", reason: "quit" });
		}
	} finally {
		session.dispose();
	}
}

function cancelRun(reason) {
	if (!activeRun || activeRun.controller.signal.aborted) return;
	activeRun.stopReason = reason;
	activeRun.controller.abort();
	// Do not await abort from an event handler: it waits for the same agent loop.
	void activeRun.session?.abort().catch(diagnostic);
}

async function run(frame) {
	const request = frame.request;
	if (!request || typeof request !== "object") throw new Error("run request is missing");
	if (typeof request.cwd !== "string" || !path.isAbsolute(request.cwd)) throw new Error("run cwd must be absolute");
	if (!request.model || typeof request.model !== "string") throw new Error("run model is missing");
	if (!request.system_prompt || typeof request.system_prompt !== "string") throw new Error("run system prompt is missing");
	if (typeof request.user_prompt !== "string") throw new Error("run user prompt is missing");
	if (!request.output_schema || request.output_schema.type !== "object") throw new Error("run output schema must describe an object");
	const tools = request.tools ?? [];
	if (!Array.isArray(tools)) throw new Error("invalid tool list");
	const names = new Set([OUTPUT_TOOL]);
	for (const tool of tools) {
		if (!tool || typeof tool.name !== "string" || typeof tool.sdk_name !== "string") throw new Error("invalid tool specification");
		if (DENIED_TOOLS.includes(tool.sdk_name) || names.has(tool.sdk_name)) throw new Error(`forbidden or duplicate Pi tool: ${tool.sdk_name}`);
		names.add(tool.sdk_name);
	}
	const controller = new AbortController();
	activeRun = { id: frame.id, controller, session: undefined, stopReason: undefined };
	const timer = request.timeout_ms == null ? undefined : setTimeout(() => cancelRun("timeout"), request.timeout_ms);
	let session;
	let unsubscribe;
	let submitted = false;
	let output;
	let outputError;
	let turns = 0;
	let toolCalls = 0;
	const usage = {};
	const started = Date.now();
	try {
		const modelRuntime = await sdk.ModelRuntime.create({
			authPath: path.join(agentDir, "auth.json"),
			modelsPath: path.join(agentDir, "models.json"),
			signal: controller.signal,
		});
		const resolved = sdk.resolveCliModel({ cliModel: request.model, modelRuntime });
		if (resolved.error || !resolved.model) throw new Error(resolved.error ?? `model not found: ${request.model}`);
		if (!modelRuntime.hasConfiguredAuth(resolved.model.provider)) throw new Error(`no configured Pi authentication for provider ${resolved.model.provider}`);
		const customTools = tools.map((tool) => ({
			name: tool.sdk_name,
			label: tool.name,
			description: tool.description,
			parameters: tool.input_schema,
			executionMode: "parallel",
			async execute(_id, params, signal) {
				const result = await invokePythonTool(tool.name, params, signal);
				if (!result.ok) throw new Error(`${result.error_code ?? "TOOL_ERROR"}: ${String(result.value ?? "tool failed")}`);
				const text = typeof result.value === "string" ? result.value : JSON.stringify(result.value);
				return { content: [{ type: "text", text }], details: { canonicalName: tool.name } };
			},
		}));
		customTools.push({
			name: OUTPUT_TOOL,
			label: "ACR result",
			description: "Submit the final ACR result matching the supplied schema. Call exactly once, after all investigation, with no other tools in the same turn.",
			parameters: request.output_schema,
			executionMode: "sequential",
			async execute(_id, params) {
				if (submitted) {
					outputError = "multiple structured results submitted";
					throw new Error(outputError);
				}
				submitted = true;
				output = params;
				return { content: [{ type: "text", text: "ACR result accepted." }], details: {}, terminate: true };
			},
		});
		const resourceLoader = new sdk.DefaultResourceLoader({
			cwd: request.cwd,
			agentDir,
			additionalExtensionPaths: request.trusted_extensions ?? [],
			noExtensions: true,
			noSkills: true,
			noPromptTemplates: true,
			noThemes: true,
			noContextFiles: true,
			systemPromptOverride: () => request.system_prompt,
			appendSystemPromptOverride: () => [],
		});
		await resourceLoader.reload();
		const extensionErrors = resourceLoader.getExtensions().errors;
		if (extensionErrors.length > 0) throw new Error(`extension load failed: ${JSON.stringify(extensionErrors)}`);
		if (typeof sdk.initTheme === "function") sdk.initTheme("dark");
		const sessionManager = request.keep_native_sessions
			? sdk.SessionManager.create(request.cwd, process.env.PI_CODING_AGENT_SESSION_DIR)
			: sdk.SessionManager.inMemory(request.cwd);
		({ session } = await sdk.createAgentSession({
			cwd: request.cwd,
			agentDir,
			modelRuntime,
			model: resolved.model,
			thinkingLevel: resolved.thinkingLevel ?? request.thinking,
			resourceLoader,
			sessionManager,
			noTools: "builtin",
			tools: [...names],
			excludeTools: DENIED_TOOLS,
			customTools,
		}));
		activeRun.session = session;
		await session.bindExtensions({ mode: "print" });
		const activeTools = session.getActiveToolNames();
		if (activeTools.length !== names.size || activeTools.some((name) => !names.has(name))) throw new Error("Pi session tool allowlist mismatch");
		controller.signal.throwIfAborted();
		unsubscribe = session.subscribe((event) => {
			if (event.type === "turn_start") turns++;
			if (event.type === "tool_execution_start") toolCalls++;
			if (event.type === "message_end" && event.message.role === "assistant") {
				const message = event.message;
				for (const [source, target] of [["input", "input"], ["output", "output"], ["cacheRead", "cache_read"], ["cacheWrite", "cache_write"]]) {
					if (typeof message.usage?.[source] === "number") usage[target] = (usage[target] ?? 0) + message.usage[source];
				}
				if (typeof message.usage?.cost?.total === "number") usage.cost = (usage.cost ?? 0) + message.usage.cost.total;
				const calls = message.content.filter((part) => part.type === "toolCall");
				if (calls.some((call) => call.name === OUTPUT_TOOL) && calls.length !== 1) outputError = "structured result must be submitted without other tool calls";
			}
			// Streamed messages and tool events use the same Python log routing.
			emit({ type: "progress", id: frame.id, event });
		});
		emit({ type: "session.started", id: frame.id, model: request.model });
		await session.prompt(request.user_prompt, { expandPromptTemplates: false });
		controller.signal.throwIfAborted();
		const lastAssistant = session.messages.filter((message) => message.role === "assistant").at(-1);
		if (lastAssistant?.stopReason === "error" || lastAssistant?.stopReason === "aborted") throw new Error(lastAssistant.errorMessage ?? `Pi agent stopped: ${lastAssistant.stopReason}`);
		if (outputError) throw new Error(outputError);
		if (!submitted) throw new Error("Pi agent did not submit a structured result");
		if (usage.input != null && usage.output != null) usage.total = usage.input + usage.output;
		Object.assign(usage, { turns, tool_calls: toolCalls, duration_ms: Date.now() - started });
		emit({ type: "result", id: frame.id, status: "completed", value: output, model: request.model, usage });
	} catch (error) {
		Object.assign(usage, { turns, tool_calls: toolCalls, duration_ms: Date.now() - started });
		emit({ type: "error", id: frame.id, status: controller.signal.aborted ? "cancelled" : "failed", error: activeRun.stopReason ?? (error instanceof Error ? error.message : String(error)), usage });
	} finally {
		clearTimeout(timer);
		unsubscribe?.();
		await shutdownSession(session);
		for (const pending of pendingTools.values()) pending.resolve({ ok: false, error_code: "BRIDGE_CLOSED" });
		pendingTools.clear();
	}
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
let receivedRun = false;
input.on("line", (line) => {
	try {
		if (Buffer.byteLength(line, "utf8") > MAX_FRAME_BYTES) throw new Error("input frame exceeds 4 MiB");
		const frame = JSON.parse(line);
		if (frame.version !== VERSION) throw new Error("incompatible protocol version");
		if (frame.type === "tool.result") return settleTool(frame);
		if (frame.type === "cancel" && frame.id === activeRun?.id) return cancelRun("cancelled by caller");
		if (frame.type !== "run" || receivedRun) throw new Error("bridge accepts exactly one run");
		receivedRun = true;
		void run(frame).catch((error) => {
			emit({ type: "error", id: frame.id, status: "failed", error: error instanceof Error ? error.message : String(error) });
		}).finally(() => {
			input.close();
			setTimeout(() => process.exit(0), 10).unref();
		});
	} catch (error) {
		cancelRun("invalid protocol frame");
		emit({ type: "error", id: activeRun?.id, status: "invalid_request", error: error instanceof Error ? error.message : String(error) });
	}
});
