"""Configuration loading for the ACR runtime.

Configuration is deliberately boring: command-line callers pass an already
parsed raw argument string, environment variables override ``acr.toml``, and
provider credentials are only ever read from an environment variable.
"""

from __future__ import annotations

import os
import tomllib
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any, Mapping


class ConfigError(ValueError):
    """A configuration value is invalid or unsafe to use."""


_TOOL_ROLES = (
    "maintainability",
    "development",
    "security",
    "hardware",
    "documentation",
    "verification",
    "consolidation",
    "summary",
    "grader",
)


@dataclass(frozen=True)
class ToolConfig:
    """Per-agent tool allowlists loaded from the ``[tools]`` table."""

    maintainability: tuple[str, ...] = ()
    development: tuple[str, ...] = ()
    security: tuple[str, ...] = ()
    hardware: tuple[str, ...] = ()
    documentation: tuple[str, ...] = ()
    verification: tuple[str, ...] = ()
    consolidation: tuple[str, ...] = ()
    summary: tuple[str, ...] = ()
    grader: tuple[str, ...] = ()

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

    def uses(self, tool_name: str) -> bool:
        return any(tool_name in self.for_agent(role) for role in _TOOL_ROLES)

    def as_dict(self) -> dict[str, list[str]]:
        return {role: list(self.for_agent(role)) for role in _TOOL_ROLES}


@dataclass(frozen=True)
class ProviderConfig:
    adapter: str = "openai"
    api_key_env: str = "OPENAI_API_KEY"
    base_url_env: str = "OPENAI_BASE_URL"
    base_url_value: str | None = None
    response_input_exclude_fields: tuple[str, ...] = ()
    retryable_http_error_types: tuple[str, ...] = ()

    @property
    def api_key(self) -> str | None:
        return os.environ.get(self.api_key_env)

    @property
    def base_url(self) -> str | None:
        value = os.environ.get(self.base_url_env, self.base_url_value or "").strip()
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
    model: str = "gpt-5.5"
    reasoning_effort: str = "high"
    wire_api: str = "responses"
    per_persona_context: str = "auto"
    max_concurrency: int = 5
    timeout_seconds: float | None = None
    max_turns: int | None = None
    retries: int = 2
    log_root: Path = field(default_factory=lambda: Path(os.environ.get("TMPDIR", "/tmp")) / "aster-code-review")
    output: Path | None = None
    overwrite: bool = False
    provider: ProviderConfig = field(default_factory=ProviderConfig)
    # Appended fields preserve positional compatibility with the original
    # runtime while allowing separate grader and tool policies.
    review_model: str | None = None
    tools_enabled: bool = True
    postprocess_reasoning_effort: str = "low"
    model_retries: int = 3
    model_retry_initial_delay: float = 1.0
    model_retry_max_delay: float = 10.0
    model_retry_multiplier: float = 2.0
    model_retry_jitter: bool = True
    # Local OpenAI Agents SDK trace settings.  These are appended to preserve
    # positional compatibility with existing integrations.
    tracing_enabled: bool = True
    trace_include_sensitive_data: bool = True
    trace_max_content_bytes: int = 65536
    web_search_context_size: str = "medium"
    benchmark_remote: str | None = None
    tools: ToolConfig = field(default_factory=ToolConfig)
    pi: PiConfig = field(default_factory=PiConfig)

    def validate(self) -> "RunConfig":
        if self.backend not in {"openai-agents", "pi-agent", "fake"}:
            raise ConfigError(f"unsupported backend: {self.backend}")
        if not self.model.strip():
            raise ConfigError("model must not be empty")
        if self.review_model is not None and not self.review_model.strip():
            raise ConfigError("review_model must not be empty")
        if self.backend == "openai-agents" and self.wire_api not in {"responses", "chat_completions"}:
            raise ConfigError("wire_api must be responses or chat_completions")
        if self.backend == "openai-agents" and self.provider.adapter not in {"openai", "any-llm"}:
            raise ConfigError(f"unsupported provider adapter: {self.provider.adapter}")
        if not self.provider.api_key_env.strip():
            raise ConfigError("provider api_key_env must not be empty")
        if not self.provider.base_url_env.strip():
            raise ConfigError("provider base_url_env must not be empty")
        if any(not field.strip() for field in self.provider.response_input_exclude_fields):
            raise ConfigError("provider response_input_exclude_fields must contain non-empty strings")
        if any(not error_type.strip() for error_type in self.provider.retryable_http_error_types):
            raise ConfigError("provider retryable_http_error_types must contain non-empty strings")
        if self.reasoning_effort not in {"low", "medium", "high", "max"}:
            raise ConfigError("reasoning_effort must be low, medium, high, or max")
        if self.postprocess_reasoning_effort not in {"low", "medium", "high", "max"}:
            raise ConfigError("postprocess_reasoning_effort must be low, medium, high, or max")
        if self.per_persona_context not in {"auto", "yes", "no"}:
            raise ConfigError("per_persona_context must be auto, yes, or no")
        if self.max_concurrency < 1:
            raise ConfigError("max_concurrency must be positive")
        if self.timeout_seconds is not None and self.timeout_seconds <= 0:
            raise ConfigError("timeout_seconds must be positive when configured")
        if self.max_turns is not None and self.max_turns < 1:
            raise ConfigError("max_turns must be positive when configured")
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
        if self.benchmark_remote is not None and not self.benchmark_remote.strip():
            raise ConfigError("benchmark_remote must not be empty")
        if (
            self.tools_enabled
            and self.backend == "openai-agents"
            and self.tools.uses("web.search")
            and self.wire_api != "responses"
        ):
            raise ConfigError(
                "hosted web search requires wire_api=responses; remove web.search from [tools] for chat_completions"
            )
        return self

    @property
    def fan_out(self) -> bool:
        """Resolve ``auto`` to the current recall-first fan-out policy."""

        return self.per_persona_context != "no"

    def with_backend(self, backend: str) -> "RunConfig":
        return replace(self, backend=backend).validate()


def _as_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if str(value).lower() in {"1", "true", "yes", "on"}:
        return True
    if str(value).lower() in {"0", "false", "no", "off"}:
        return False
    raise ConfigError(f"invalid boolean value: {value!r}")


def _as_optional_positive_int(value: Any) -> int | None:
    if value is None:
        return None
    if isinstance(value, str) and value.strip().lower() in {"", "none", "unlimited", "off"}:
        return None
    try:
        return int(value)
    except (TypeError, ValueError) as exc:
        raise ConfigError(f"invalid integer value: {value!r}") from exc


def _as_optional_positive_float(value: Any) -> float | None:
    if value is None:
        return None
    if isinstance(value, str) and value.strip().lower() in {"", "none", "unlimited", "off"}:
        return None
    try:
        return float(value)
    except (TypeError, ValueError) as exc:
        raise ConfigError(f"invalid float value: {value!r}") from exc


def _as_string_tuple(value: Any, *, name: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        return tuple(item.strip() for item in value.split(",") if item.strip())
    if not isinstance(value, list | tuple) or any(not isinstance(item, str) for item in value):
        raise ConfigError(f"{name} must be an array of strings")
    return tuple(value)


def _tool_config(value: Any) -> ToolConfig:
    if value is None:
        return ToolConfig()
    if not isinstance(value, Mapping):
        raise ConfigError("tools must be a TOML table")
    unknown = set(value) - set(_TOOL_ROLES)
    if unknown:
        raise ConfigError("unknown tool roles: " + ", ".join(sorted(unknown)))
    return ToolConfig(
        **{
            role: _as_string_tuple(value.get(role), name=f"tools.{role}")
            for role in _TOOL_ROLES
        }
    ).validate()


def _read_toml(path: Path | None, *, required: bool = False) -> dict[str, Any]:
    if path is None:
        return {}
    try:
        with path.open("rb") as stream:
            value = tomllib.load(stream)
    except FileNotFoundError as exc:
        if required:
            raise ConfigError(f"config file does not exist: {path}") from exc
        return {}
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"cannot read config {path}: {exc}") from exc
    except OSError as exc:
        raise ConfigError(f"cannot read config {path}: {exc}") from exc
    return value


def load_config(path: str | Path | None = None, *, overrides: Mapping[str, Any] | None = None) -> RunConfig:
    """Load config using CLI-style overrides > environment > TOML > defaults."""

    if path is not None:
        config_path = Path(path)
    else:
        local = Path.cwd() / "acr.toml"
        config_path = local if local.exists() else Path(__file__).resolve().with_name("acr.toml")
    # An explicitly selected file is authoritative and must exist. Never fall
    # back to, scan for, or merge another TOML when --config was supplied.
    document = _read_toml(config_path, required=path is not None)
    agent = dict(document.get("agent", {}))
    provider = dict(document.get("provider", {}))
    tracing = dict(document.get("tracing", {}))
    benchmark = dict(document.get("benchmark", {}))
    pi = dict(document.get("pi", {}))
    tools = _tool_config(document.get("tools"))

    def value(name: str, env: str, default: Any) -> Any:
        if overrides and name in overrides and overrides[name] is not None:
            return overrides[name]
        if env in os.environ:
            return os.environ[env]
        return agent.get(name, default)

    provider_cfg = ProviderConfig(
        adapter=str(os.environ.get("ACR_PROVIDER_ADAPTER", provider.get("adapter", "openai"))),
        api_key_env=str(provider.get("api_key_env", "OPENAI_API_KEY")),
        base_url_env=str(provider.get("base_url_env", "OPENAI_BASE_URL")),
        base_url_value=str(provider.get("base_url", provider.get("base_url_value", ""))) or None,
        response_input_exclude_fields=_as_string_tuple(
            provider.get("response_input_exclude_fields"),
            name="provider.response_input_exclude_fields",
        ),
        retryable_http_error_types=_as_string_tuple(
            provider.get("retryable_http_error_types"),
            name="provider.retryable_http_error_types",
        ),
    )
    if overrides:
        provider_cfg = ProviderConfig(
            adapter=str(overrides.get("provider_adapter", provider_cfg.adapter)),
            api_key_env=str(overrides.get("api_key_env", provider_cfg.api_key_env)),
            base_url_env=str(overrides.get("base_url_env", provider_cfg.base_url_env)),
            base_url_value=provider_cfg.base_url_value,
            response_input_exclude_fields=provider_cfg.response_input_exclude_fields,
            retryable_http_error_types=provider_cfg.retryable_http_error_types,
        )
    pi_cfg = PiConfig(
        node_command=str(os.environ.get("ACR_PI_NODE_COMMAND", pi.get("node_command", "node"))),
        agent_dir=Path(
            str(os.environ.get("ACR_PI_AGENT_DIR", pi.get("agent_dir", "~/.pi/agent")))
        ).expanduser(),
        bridge_startup_timeout_seconds=float(
            pi.get("bridge_startup_timeout_seconds", 30.0)
        ),
        shutdown_grace_seconds=float(pi.get("shutdown_grace_seconds", 5.0)),
        keep_native_sessions=_as_bool(pi.get("keep_native_sessions", True)),
        trusted_extensions=_as_string_tuple(
            pi.get("trusted_extensions"), name="pi.trusted_extensions"
        ),
    ).validate()
    cfg = RunConfig(
        backend=str(value("backend", "ACR_BACKEND", "openai-agents")),
        model=str(value("model", "ACR_MODEL", "gpt-5.5")),
        review_model=str(value("review_model", "ACR_REVIEW_MODEL", agent.get("model", "gpt-5.5"))),
        reasoning_effort=str(value("reasoning_effort", "ACR_REASONING_EFFORT", "high")),
        postprocess_reasoning_effort=str(
            value("postprocess_reasoning_effort", "ACR_POSTPROCESS_REASONING_EFFORT", "low")
        ),
        wire_api=str(value("wire_api", "ACR_WIRE_API", "responses")),
        per_persona_context=str(value("per_persona_context", "ACR_PER_PERSONA_CONTEXT", "auto")),
        max_concurrency=int(value("max_concurrency", "ACR_MAX_CONCURRENCY", 5)),
        timeout_seconds=_as_optional_positive_float(
            value("timeout_seconds", "ACR_TIMEOUT_SECONDS", None)
        ),
        max_turns=_as_optional_positive_int(value("max_turns", "ACR_MAX_TURNS", None)),
        retries=int(value("retries", "ACR_RETRIES", 2)),
        model_retries=int(value("model_retries", "ACR_MODEL_RETRIES", 3)),
        model_retry_initial_delay=float(
            value("model_retry_initial_delay", "ACR_MODEL_RETRY_INITIAL_DELAY", 1.0)
        ),
        model_retry_max_delay=float(
            value("model_retry_max_delay", "ACR_MODEL_RETRY_MAX_DELAY", 10.0)
        ),
        model_retry_multiplier=float(
            value("model_retry_multiplier", "ACR_MODEL_RETRY_MULTIPLIER", 2.0)
        ),
        model_retry_jitter=_as_bool(
            value("model_retry_jitter", "ACR_MODEL_RETRY_JITTER", True)
        ),
        tracing_enabled=_as_bool(
            os.environ.get("ACR_TRACING_ENABLED", tracing.get("enabled", True))
        ),
        trace_include_sensitive_data=_as_bool(
            os.environ.get(
                "ACR_TRACE_INCLUDE_SENSITIVE_DATA",
                tracing.get("include_sensitive_data", True),
            )
        ),
        trace_max_content_bytes=int(
            os.environ.get(
                "ACR_TRACE_MAX_CONTENT_BYTES",
                tracing.get("max_content_bytes", 65536),
            )
        ),
        tools_enabled=_as_bool(value("tools_enabled", "ACR_ENABLE_TOOLS", True)),
        tools=tools,
        web_search_context_size=str(
            value(
                "web_search_context_size",
                "ACR_WEB_SEARCH_CONTEXT_SIZE",
                "medium",
            )
        ),
        log_root=Path(value("log_root", "ACR_LOG_ROOT", str(Path(os.environ.get("TMPDIR", "/tmp")) / "aster-code-review"))),
        output=Path(value("output", "ACR_OUTPUT", "")) if value("output", "ACR_OUTPUT", "") else None,
        overwrite=_as_bool(value("overwrite", "ACR_OVERWRITE", False)),
        provider=provider_cfg,
        pi=pi_cfg,
        benchmark_remote=(
            str(benchmark["remote"]).strip() if "remote" in benchmark else None
        ),
    )
    return cfg.validate()
