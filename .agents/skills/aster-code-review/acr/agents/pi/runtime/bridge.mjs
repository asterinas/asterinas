#!/usr/bin/env node

// ACR's Pi Agent SDK subprocess bridge.

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
const TOOL_RUNTIME = Symbol.for("acr.pi.tool-runtime.v1");
const REGISTER_EVENT = "pi-subagents:runtime-agent-register:v1";
const REQUEST_EVENT = "prompt-template:subagent:request";
const STARTED_EVENT = "prompt-template:subagent:started";
const UPDATE_EVENT = "prompt-template:subagent:update";
const RESPONSE_EVENT = "prompt-template:subagent:response";
const CANCEL_EVENT = "prompt-template:subagent:cancel";

function emit(value) {
	protocolWrite(`${JSON.stringify({ version: VERSION, source_timestamp: new Date().toISOString(), ...value })}\n`);
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
const subagentsRoot = path.join(agentDir, "npm", "node_modules", "pi-subagents");
const subagentsPath = path.join(subagentsRoot, "index.ts");
const subagentsManifest = JSON.parse(fs.readFileSync(path.join(subagentsRoot, "package.json"), "utf8"));
const toolExtension = path.resolve(process.env.ACR_PI_TOOL_EXTENSION ?? path.join(path.dirname(new URL(import.meta.url).pathname), "tool_extension.ts"));
const guidelineScript = path.resolve(path.dirname(toolExtension), "..", "scripts", "print_guideline.py");

emit({
	type: "ready",
	bridge_version: "1",
	pi_version: sdkManifest.version,
	pi_subagents_version: subagentsManifest.version,
	guideline_script: guidelineScript,
	capabilities: [
		"structured_delegation",
		"runtime_agent_registration",
		"usage",
		"tool_proxy",
		"cancellation",
	],
});

let activeRun;
const pendingTools = new Map();

function settleTool(frame) {
	const pending = pendingTools.get(frame.call_id);
	if (!pending || frame.run_id !== activeRun?.id) return;
	pendingTools.delete(frame.call_id);
	pending.resolve({ ok: frame.ok === true, value: frame.value, error_code: frame.error_code });
}

function invokePythonTool(tool, argumentsValue, signal) {
	const callId = randomUUID();
	emit({ type: "tool.call", run_id: activeRun.id, call_id: callId, tool, arguments: argumentsValue });
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

async function run(requestFrame) {
	if (activeRun) throw new Error("bridge accepts exactly one run");
	const request = requestFrame.request;
	if (!request || typeof request !== "object") throw new Error("run request is missing");
	if (typeof request.cwd !== "string" || !path.isAbsolute(request.cwd)) throw new Error("run cwd must be absolute");
	if (!request.model || typeof request.model !== "string") throw new Error("run model is missing");
	if (!request.system_prompt || typeof request.system_prompt !== "string") throw new Error("run system prompt is missing");
	if (typeof request.user_prompt !== "string") throw new Error("run user prompt is missing");
	if (!request.output_schema || typeof request.output_schema !== "object") throw new Error("run output schema is missing");
	const tools = Array.isArray(request.tools) ? request.tools : [];
	for (const tool of tools) {
		if (!tool || typeof tool.name !== "string" || typeof tool.sdk_name !== "string") throw new Error("invalid tool specification");
		if (["write", "edit", "bash", "powershell", "subagent", "contact_supervisor", "intercom"].includes(tool.sdk_name)) {
			throw new Error(`forbidden Pi tool: ${tool.sdk_name}`);
		}
	}
	const controller = new AbortController();
	activeRun = { id: requestFrame.id, controller, pi: undefined, response: undefined };
	let toolAcknowledged = tools.length === 0;
	globalThis[TOOL_RUNTIME] = {
		runId: requestFrame.id,
		tools,
		invoke: invokePythonTool,
		acknowledge() { toolAcknowledged = true; },
	};
	let session;
	let registration;
	try {
		const authPath = path.join(agentDir, "auth.json");
		const modelsPath = path.join(agentDir, "models.json");
		const modelRuntime = await sdk.ModelRuntime.create({ authPath, modelsPath });
		const resolved = sdk.resolveCliModel({ cliModel: request.model, modelRuntime });
		if (resolved.error || !resolved.model) throw new Error(resolved.error ?? `model not found: ${request.model}`);
		if (!modelRuntime.hasConfiguredAuth(resolved.model.provider)) {
			throw new Error(`no configured Pi authentication for provider ${resolved.model.provider}`);
		}
		const childName = `acr-${request.role}-${randomUUID().slice(0, 12)}`;
		let hostPi;
		let terminalResolve;
		let terminalReject;
		const terminal = new Promise((resolve, reject) => {
			terminalResolve = resolve;
			terminalReject = reject;
		});
		const hostExtension = {
			name: "acr-pi-host",
			factory(pi) {
				hostPi = pi;
				pi.events.on(STARTED_EVENT, (event) => {
					if (event?.requestId === requestFrame.id) emit({ type: "delegation.started", id: requestFrame.id, ...event });
				});
				pi.events.on(UPDATE_EVENT, (event) => {
					if (event?.requestId === requestFrame.id) emit({ type: "progress", id: requestFrame.id, ...event });
				});
				pi.events.on(RESPONSE_EVENT, (event) => {
					if (event?.requestId === requestFrame.id) terminalResolve(event);
				});
				pi.on("session_start", () => {
					const registrationRequest = {
						version: 1,
						name: childName,
						definition: {
							description: `ACR ${request.role} reviewer`,
							systemPrompt: request.system_prompt,
							tools: tools.map((tool) => tool.sdk_name),
							excludeTools: ["read", "bash", "edit", "write", "powershell", "subagent", "contact_supervisor", "intercom"],
							allowNestedSubagents: false,
							model: request.model,
							thinking: request.thinking,
							systemPromptMode: "replace",
							inheritProjectContext: false,
							inheritGlobalContext: false,
							inheritSkills: false,
							defaultContext: "fresh",
							defaultTimeoutMs: request.timeout_ms ?? undefined,
							defaultToolTimeoutMs: request.tool_timeout_ms,
							acceptanceRole: "read-only",
							runner: { type: "pi" },
							extensions: Array.isArray(request.trusted_extensions) ? request.trusted_extensions : [],
							subagentOnlyExtensions: tools.length > 0 ? [toolExtension] : [],
							mutationTools: [],
							maxSubagentDepth: 1,
							completionGuard: false,
						},
					};
					pi.events.emit(REGISTER_EVENT, registrationRequest);
					if (registrationRequest.result?.ok !== true) {
						terminalReject(registrationRequest.result?.error ?? new Error("runtime agent registration failed"));
						return;
					}
					registration = registrationRequest.result.registration;
				});
			},
		};
		const eventBus = sdk.createEventBus();
		const resourceLoader = new sdk.DefaultResourceLoader({
			cwd: request.cwd,
			agentDir,
			eventBus,
			additionalExtensionPaths: [subagentsPath],
			extensionFactories: [hostExtension],
			noExtensions: true,
			noSkills: true,
			noPromptTemplates: true,
			noThemes: true,
			noContextFiles: true,
		});
		await resourceLoader.reload();
		const extensionErrors = resourceLoader.getExtensions().errors;
		if (extensionErrors.length > 0) throw new Error(`extension load failed: ${JSON.stringify(extensionErrors)}`);
		if (typeof sdk.initTheme === "function") sdk.initTheme("dark");
		({ session } = await sdk.createAgentSession({
			cwd: request.cwd,
			agentDir,
			modelRuntime,
			model: resolved.model,
			thinkingLevel: resolved.thinkingLevel ?? request.thinking,
			resourceLoader,
			sessionManager: sdk.SessionManager.inMemory(request.cwd),
			noTools: "all",
		}));
		activeRun.pi = hostPi;
		await session.bindExtensions({ mode: "print" });
		if (!registration) throw new Error("Pi runtime child was not registered");
		const delegationRequest = {
			requestId: requestFrame.id,
			ownerRunId: requestFrame.id,
			nodeId: "acr-child",
			agent: childName,
			task: request.user_prompt,
			context: "fresh",
			cwd: request.cwd,
			model: request.model,
			thinking: request.thinking,
			timeoutMs: request.timeout_ms ?? undefined,
			skill: false,
			artifacts: request.keep_native_sessions === true,
			result: { kind: "structured", schema: request.output_schema },
		};
		hostPi.events.emit(REQUEST_EVENT, delegationRequest);
		const response = await terminal;
		if (tools.length > 0 && !toolAcknowledged) throw new Error("ACR child tool extension did not start");
		const usage = response.usage
			? {
				input: response.usage.input,
				output: response.usage.output,
				cache_read: response.usage.cacheRead,
				cache_write: response.usage.cacheWrite,
				total: response.usage.input + response.usage.output,
				cost: response.usage.cost,
				turns: response.usage.turns,
				tool_calls: response.usage.toolCalls,
				duration_ms: response.usage.durationMs,
			}
			: undefined;
		if (response.status !== "completed" || response.result?.kind !== "structured") {
			emit({ type: "error", id: requestFrame.id, status: response.status, error: response.error ?? "Pi child failed", usage });
		} else {
			emit({
				type: "result",
				id: requestFrame.id,
				status: "completed",
				value: response.result.value,
				model: response.model,
				launch_contract_digest: response.launchContractDigest,
				usage,
			});
		}
	} finally {
		registration?.dispose();
		await shutdownSession(session);
		delete globalThis[TOOL_RUNTIME];
		for (const pending of pendingTools.values()) pending.resolve({ ok: false, error_code: "BRIDGE_CLOSED" });
		pendingTools.clear();
	}
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", (line) => {
	if (Buffer.byteLength(line, "utf8") > MAX_FRAME_BYTES) {
		emit({ type: "error", id: activeRun?.id, status: "invalid_request", error: "input frame exceeds 4 MiB" });
		return;
	}
	let frame;
	try {
		frame = JSON.parse(line);
	} catch {
		emit({ type: "error", id: activeRun?.id, status: "invalid_request", error: "input is not JSON" });
		return;
	}
	if (frame.version !== VERSION) return;
	if (frame.type === "tool.result") return settleTool(frame);
	if (frame.type === "cancel" && frame.id === activeRun?.id) {
		activeRun.controller.abort();
		activeRun.pi?.events.emit(CANCEL_EVENT, { requestId: frame.id, ownerRunId: frame.id, nodeId: "acr-child" });
		return;
	}
	if (frame.type === "run") {
		void run(frame).catch((error) => {
			emit({ type: "error", id: frame.id, status: "failed", error: error instanceof Error ? error.message : String(error) });
		}).finally(() => {
			input.close();
			setTimeout(() => process.exit(0), 10).unref();
		});
	}
});
