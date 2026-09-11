"""Configuration loading for the ACR runtime.

Every user-facing setting is declared once in ``_SETTINGS`` with its profile
key and its environment variable. A setting's value comes from the first
source that provides it:

1. a command-line override (``--per-persona-context``);
2. its ``ACR_*`` environment variable;
3. the selected TOML profile;
4. the built-in default on ``RunConfig``.

The backend is the exception: models and backend tables only make sense for
the backend they were written for, so only the profile selects it.

Internal policy, such as each agent role's tool allowlist, is not configurable.
Credentials come from environment variables or an SDK's own login store.
"""

from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, Callable, Mapping


class ConfigError(ValueError):
    """A configuration value is invalid or unsafe to use."""


_PERSONA_ROLES = (
    "maintainability",
    "development",
    "security",
    "hardware",
    "documentation",
)
_TOOL_ROLES = (
    *_PERSONA_ROLES,
    "verification",
    "consolidation",
    "summary",
    "grader",
)

# Built-in tool allowlists. Personas read source, history, and guidelines;
# verification also lists files; the remaining roles work only on their input.
_PERSONA_TOOLS = ("source.read", "source.search", "git.show", "git.log", "git.blame", "guideline.show")
_VERIFICATION_TOOLS = ("source.read", "source.search", "source.list", "git.show", "git.log", "git.blame")
# Roles that receive hosted web search on the openai-agents backend with the
# Responses API.
_WEB_SEARCH_ROLES = ("maintainability", "development", "verification")


@dataclass(frozen=True)
class ToolConfig:
    """Per-agent tool allowlists."""

    maintainability: tuple[str, ...] = ()
    development: tuple[str, ...] = ()
    security: tuple[str, ...] = ()
    hardware: tuple[str, ...] = ()
    documentation: tuple[str, ...] = ()
    verification: tuple[str, ...] = ()
    consolidation: tuple[str, ...] = ()
    summary: tuple[str, ...] = ()
    grader: tuple[str, ...] = ()

    @classmethod
    def builtin(cls, *, web_search: bool = False) -> "ToolConfig":
        """Return ACR's allowlists, optionally adding hosted web search."""

        roles: dict[str, tuple[str, ...]] = {
            "maintainability": _PERSONA_TOOLS,
            "development": _PERSONA_TOOLS,
            "security": _PERSONA_TOOLS,
            "hardware": _PERSONA_TOOLS,
            "documentation": _PERSONA_TOOLS,
            "verification": _VERIFICATION_TOOLS,
        }
        if web_search:
            for role in _WEB_SEARCH_ROLES:
                roles[role] = (*roles[role], "web.search")
        return cls(**roles)

    def validate(self) -> "ToolConfig":
        for role in _TOOL_ROLES:
            names = self.for_agent(role)
            if any(not isinstance(name, str) or not name.strip() for name in names):
                raise ConfigError(f"tools.{role} must contain non-empty strings")
            if any(name != name.strip() for name in names):
                raise ConfigError(f"tools.{role} entries must not contain surrounding whitespace")
            if len(set(names)) != len(names):
                raise ConfigError(f"tools.{role} must not contain duplicates")
        return self

    def for_agent(self, role: str) -> tuple[str, ...]:
        if role not in _TOOL_ROLES:
            raise ConfigError(f"unknown tool role: {role}")
        return getattr(self, role)

    def roles_using(self, tool_name: str) -> tuple[str, ...]:
        return tuple(role for role in _TOOL_ROLES if tool_name in self.for_agent(role))

    def without(self, tool_name: str, roles: tuple[str, ...]) -> "ToolConfig":
        """Return a copy with ``tool_name`` removed from ``roles``."""

        return replace(self, **{
            role: tuple(name for name in self.for_agent(role) if name != tool_name)
            for role in roles
        })

    def uses(self, tool_name: str) -> bool:
        return any(tool_name in self.for_agent(role) for role in _TOOL_ROLES)

    def as_dict(self) -> dict[str, list[str]]:
        return {role: list(self.for_agent(role)) for role in _TOOL_ROLES}


@dataclass(frozen=True)
class ProviderConfig:
    adapter: str = "openai"
    # Fixed for users; tests point them at isolated variables.
    api_key_env: str = "ACR_API_KEY"
    base_url_env: str = "ACR_BASE_URL"

    @property
    def api_key(self) -> str | None:
        return os.environ.get(self.api_key_env)

    @property
    def base_url(self) -> str | None:
        value = os.environ.get(self.base_url_env, "").strip()
        return value or None


@dataclass(frozen=True)
class PiConfig:
    """Pi SDK subprocess settings; authentication remains owned by Pi."""

    node_command: str = "node"
    agent_dir: Path = field(default_factory=lambda: Path("~/.pi/agent").expanduser())
    bridge_startup_timeout_seconds: float = 30.0
    shutdown_grace_seconds: float = 5.0
    keep_native_sessions: bool = True
    trusted_extensions: tuple[str, ...] = ()

    def validate(self) -> "PiConfig":
        if not self.node_command.strip():
            raise ConfigError("pi.node_command must not be empty")
        if self.bridge_startup_timeout_seconds <= 0:
            raise ConfigError("pi.bridge_startup_timeout_seconds must be positive")
        if self.shutdown_grace_seconds <= 0:
            raise ConfigError("pi.shutdown_grace_seconds must be positive")
        if any(not extension.strip() for extension in self.trusted_extensions):
            raise ConfigError("pi.trusted_extensions must contain non-empty strings")
        if len(set(self.trusted_extensions)) != len(self.trusted_extensions):
            raise ConfigError("pi.trusted_extensions must not contain duplicates")
        return self


@dataclass(frozen=True)
class RunConfig:
    backend: str = "openai-agents"
    review_model: str = "gpt-5.5"
    postprocess_model: str = "gpt-5.5"
    reasoning_effort: str = "high"
    wire_api: str = "responses"
    per_persona_context: str = "auto"
    max_concurrency: int = 5
    # No wall-clock limit per agent attempt.
    timeout_seconds: float | None = None
    # Retries in addition to the first attempt: at most three runs per agent.
    retries: int = 2
    log_root: Path = field(default_factory=lambda: Path(os.environ.get("TMPDIR", "/tmp")) / "aster-code-review")
    provider: ProviderConfig = field(default_factory=ProviderConfig)
    tools_enabled: bool = True
    postprocess_reasoning_effort: str = "low"
    model_retries: int = 6
    model_retry_initial_delay: float = 1.0
    model_retry_max_delay: float = 10.0
    model_retry_multiplier: float = 2.0
    model_retry_jitter: bool = True
    # Local logging controls; only tracing_enabled is OpenAI-specific.
    tracing_enabled: bool = True
    trace_max_content_bytes: int = 65536
    web_search_context_size: str = "medium"
    tools: ToolConfig = field(default_factory=ToolConfig.builtin)
    pi: PiConfig = field(default_factory=PiConfig)

    def validate(self) -> "RunConfig":
        if self.backend not in _BACKENDS:
            raise ConfigError(f"unsupported backend: {self.backend}")
        if not self.review_model.strip():
            raise ConfigError("agent.persona_review.model must not be empty")
        if not self.postprocess_model.strip():
            raise ConfigError("agent.postprocess.model must not be empty")
        if self.backend == "openai-agents" and self.wire_api not in {"responses", "chat_completions"}:
            raise ConfigError("openai.wire_api must be responses or chat_completions")
        if self.backend == "openai-agents" and self.provider.adapter not in {"openai", "any-llm"}:
            raise ConfigError(f"unsupported openai.adapter: {self.provider.adapter}")
        if not self.provider.api_key_env.strip():
            raise ConfigError("provider api_key_env must not be empty")
        if not self.provider.base_url_env.strip():
            raise ConfigError("provider base_url_env must not be empty")
        if self.reasoning_effort not in _EFFORTS:
            raise ConfigError("agent.persona_review.effort must be low, medium, high, or max")
        if self.postprocess_reasoning_effort not in _EFFORTS:
            raise ConfigError("agent.postprocess.effort must be low, medium, high, or max")
        if self.per_persona_context not in {"auto", "yes", "no"}:
            raise ConfigError("per_persona_context must be auto, yes, or no")
        if self.max_concurrency < 1:
            raise ConfigError("max_concurrency must be positive")
        if self.timeout_seconds is not None and self.timeout_seconds <= 0:
            raise ConfigError("timeout_seconds must be positive when configured")
        if self.retries < 0:
            raise ConfigError("retries cannot be negative")
        if self.model_retries < 0:
            raise ConfigError("model_retries cannot be negative")
        if self.model_retry_initial_delay < 0:
            raise ConfigError("model_retry_initial_delay cannot be negative")
        if self.model_retry_max_delay < self.model_retry_initial_delay:
            raise ConfigError("model_retry_max_delay must be at least model_retry_initial_delay")
        if self.model_retry_multiplier < 0:
            raise ConfigError("model_retry_multiplier cannot be negative")
        if self.trace_max_content_bytes < 256:
            raise ConfigError("trace_max_content_bytes must be at least 256")
        if self.web_search_context_size not in {"low", "medium", "high"}:
            raise ConfigError("web_search_context_size must be low, medium, or high")
        self.tools.validate()
        self.pi.validate()
        if (
            self.tools_enabled
            and self.backend == "openai-agents"
            and self.tools.uses("web.search")
            and self.wire_api != "responses"
        ):
            raise ConfigError("hosted web search requires openai.wire_api = \"responses\"")
        return self

    def model_for(self, role: str) -> str:
        """Select the review model for personas and the post-processing model otherwise."""

        return self.review_model if role == "persona" else self.postprocess_model

    def effort_for(self, role: str) -> str:
        """Select the reasoning effort by the same split as ``model_for``."""

        return self.reasoning_effort if role == "persona" else self.postprocess_reasoning_effort

    @staticmethod
    def stage_role(tool_role: str) -> str:
        """Map a tool-allowlist role to its agent role: personas share ``persona``."""

        return "persona" if tool_role in _PERSONA_ROLES else tool_role

    @property
    def fan_out(self) -> bool:
        """Resolve ``auto`` to the current recall-first fan-out policy."""

        return self.per_persona_context != "no"


_BACKENDS = ("openai-agents", "pi-agent", "claude-agent", "fake")
_EFFORTS = {"low", "medium", "high", "max"}

# The profile used when no ``--config`` is given.
DEFAULT_PROFILE = Path(__file__).resolve().parents[1] / "acr.openai.toml"


def _as_string_tuple(value: Any) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        return tuple(item.strip() for item in value.split(",") if item.strip())
    if not isinstance(value, list | tuple) or any(not isinstance(item, str) for item in value):
        raise ConfigError("must be an array of strings")
    return tuple(value)


def _as_str(value: Any) -> str:
    if not isinstance(value, str):
        raise ConfigError(f"must be a string, not {value!r}")
    return value


def _as_path(value: Any) -> Path:
    return Path(_as_str(value)).expanduser()


@dataclass(frozen=True)
class _Setting:
    """One user-facing setting: where it lives in a profile and in the environment."""

    key: str
    target: str
    parse: Callable[[Any], Any]
    from_env: bool = True

    @property
    def env(self) -> str | None:
        """``ACR_`` + the key without ``agent.``, upper-cased, dots as underscores."""

        if not self.from_env:
            return None
        return "ACR_" + self.key.removeprefix("agent.").replace(".", "_").upper()


_SETTINGS = (
    _Setting("agent.backend", "backend", _as_str, from_env=False),
    _Setting("agent.per_persona_context", "per_persona_context", _as_str),
    _Setting("agent.log_root", "log_root", _as_path),
    _Setting("agent.persona_review.model", "review_model", _as_str),
    _Setting("agent.persona_review.effort", "reasoning_effort", _as_str),
    _Setting("agent.postprocess.model", "postprocess_model", _as_str),
    _Setting("agent.postprocess.effort", "postprocess_reasoning_effort", _as_str),
    _Setting("openai.adapter", "provider.adapter", _as_str),
    _Setting("openai.wire_api", "wire_api", _as_str),
    _Setting("pi.agent_dir", "pi.agent_dir", _as_path),
    _Setting("pi.node_command", "pi.node_command", _as_str),
    _Setting("pi.trusted_extensions", "pi.trusted_extensions", _as_string_tuple),
)
_SETTINGS_BY_KEY = {setting.key: setting for setting in _SETTINGS}
_TABLES = {"agent", "agent.persona_review", "agent.postprocess", "openai", "pi"}

_BUILTIN_REQUEST_RETRIES = "built in: a failed model request is retried up to 6 times"
# Keys from earlier profile layouts, with the reason they are rejected.
_RETIRED_KEYS = {
    "agent.model": "use agent.persona_review.model and agent.postprocess.model",
    "agent.review_model": "moved to agent.persona_review.model",
    "agent.reasoning_effort": "moved to agent.persona_review.effort",
    "agent.postprocess_model": "moved to agent.postprocess.model",
    "agent.postprocess_reasoning_effort": "moved to agent.postprocess.effort",
    "agent.wire_api": "moved to openai.wire_api",
    "agent.max_concurrency": "built in: at most 5 agents run at once",
    "agent.retries": "built in: each agent runs at most 3 times",
    "agent.timeout_seconds": "built in: agents have no time limit",
    "agent.model_retries": _BUILTIN_REQUEST_RETRIES,
    "openai.model_retries": _BUILTIN_REQUEST_RETRIES,
    "openai.web_search": "built in: enabled whenever openai.wire_api is \"responses\"",
    "openai.api_key_env": "the API key is always read from ACR_API_KEY",
    "openai.base_url_env": "the endpoint is always read from ACR_BASE_URL",
    "provider": "renamed to [openai]",
    "tools": "tool allowlists are built in",
    "benchmark": "the benchmark fetches missing commits from the upstream repository",
    "claude": "Claude inherits the local CLI configuration",
}
_RETIRED_ENV = {
    "ACR_MODEL": "ACR_PERSONA_REVIEW_MODEL and ACR_POSTPROCESS_MODEL",
    "ACR_REVIEW_MODEL": "ACR_PERSONA_REVIEW_MODEL",
    "ACR_REASONING_EFFORT": "ACR_PERSONA_REVIEW_EFFORT",
    "ACR_POSTPROCESS_REASONING_EFFORT": "ACR_POSTPROCESS_EFFORT",
    "ACR_WIRE_API": "ACR_OPENAI_WIRE_API",
    "ACR_PROVIDER_ADAPTER": "ACR_OPENAI_ADAPTER",
    "ACR_BACKEND": "--config with a profile for that backend",
}
# Variables of settings that are now built in.
_BUILTIN_ENV = ("ACR_MAX_CONCURRENCY", "ACR_RETRIES", "ACR_TIMEOUT_SECONDS", "ACR_MODEL_RETRIES")


def _read_toml(path: Path, *, required: bool) -> dict[str, Any]:
    try:
        with path.open("rb") as stream:
            return tomllib.load(stream)
    except FileNotFoundError as exc:
        if required:
            raise ConfigError(f"config file does not exist: {path}") from exc
        return {}
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"cannot read config {path}: {exc}") from exc
    except OSError as exc:
        raise ConfigError(f"cannot read config {path}: {exc}") from exc


def _flatten(document: Mapping[str, Any], path: Path) -> dict[str, Any]:
    """Map dotted profile keys to values, rejecting anything not in the schema."""

    values: dict[str, Any] = {}

    def visit(table: Mapping[str, Any], prefix: str) -> None:
        for name, value in table.items():
            key = f"{prefix}{name}"
            if key in _RETIRED_KEYS:
                raise ConfigError(f"{path}: {key} is no longer supported: {_RETIRED_KEYS[key]}")
            if key in _TABLES:
                if not isinstance(value, Mapping):
                    raise ConfigError(f"{path}: {key} must be a table")
                visit(value, f"{key}.")
            elif key in _SETTINGS_BY_KEY:
                values[key] = value
            else:
                raise ConfigError(f"{path}: unknown setting {key}")

    visit(document, "")
    return values


def _parse(setting: _Setting, value: Any, source: str) -> Any:
    try:
        return setting.parse(value)
    except ConfigError as exc:
        raise ConfigError(f"{source}: {exc}") from exc


def load_config(path: str | Path | None = None, *, overrides: Mapping[str, Any] | None = None) -> RunConfig:
    """Load config using overrides > environment > profile > built-in defaults.

    ``overrides`` maps profile keys such as ``agent.per_persona_context`` to
    command-line values; ``None`` values are ignored. Without ``path``,
    ``DEFAULT_PROFILE`` is used.
    """

    overrides = {key: value for key, value in (overrides or {}).items() if value is not None}
    unknown = set(overrides) - set(_SETTINGS_BY_KEY)
    if unknown:
        raise ConfigError("unknown override: " + ", ".join(sorted(unknown)))
    for name, replacement in _RETIRED_ENV.items():
        if name in os.environ:
            raise ConfigError(f"{name} is no longer supported; use {replacement}")
    for name in _BUILTIN_ENV:
        if name in os.environ:
            raise ConfigError(f"{name} is no longer supported; the setting is built in")

    config_path = Path(path) if path is not None else DEFAULT_PROFILE
    # The selected file is authoritative and must exist. Never fall back to,
    # scan for, or merge another TOML.
    profile = _flatten(_read_toml(config_path, required=True), config_path)

    resolved: dict[str, Any] = {}
    for setting in _SETTINGS:
        if setting.key in overrides:
            value = _parse(setting, overrides[setting.key], f"override {setting.key}")
        elif setting.env is not None and setting.env in os.environ:
            value = _parse(setting, os.environ[setting.env], setting.env)
        elif setting.key in profile:
            value = _parse(setting, profile[setting.key], f"{config_path}: {setting.key}")
        else:
            continue
        resolved[setting.target] = value

    provider = {key.removeprefix("provider."): value for key, value in resolved.items() if key.startswith("provider.")}
    pi = {key.removeprefix("pi."): value for key, value in resolved.items() if key.startswith("pi.")}
    top = {key: value for key, value in resolved.items() if "." not in key}
    backend = top.get("backend", RunConfig.backend)
    wire_api = top.get("wire_api", RunConfig.wire_api)
    # Hosted web search exists only on the openai-agents backend's Responses API.
    web_search = backend == "openai-agents" and wire_api == "responses"
    return RunConfig(
        **top,
        provider=ProviderConfig(**provider),
        pi=PiConfig(**pi),
        tools=ToolConfig.builtin(web_search=web_search),
    ).validate()
