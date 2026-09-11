"""The explicit ACR production state machine."""

from __future__ import annotations

import asyncio
import os
import sys
from datetime import date
from pathlib import Path

from ..agents.backend_factory import close_backend, create_backend
from ..agents.factories import AgentFactory
from ..agents.openai_agents import sdk_trace_context
from ..agents.protocol import AgentBackend
from ..config import RunConfig
from ..stages.assemble import ReviewDocument, assemble_fragments
from ..stages.consolidate import consolidate_document
from ..stages.prompts import InstructionCompiler, build_prompts
from ..stages.publish import publish
from ..stages.resolve import resolve_target
from ..stages.summary import summarize_document
from ..stages.verify import verify_document
from .activation import DeterministicActivationPolicy
from .console import print_artifact
from .events import EventLogger
from .scheduler import PassScheduler
from .state import RunContext, Stage


class OrchestrationError(RuntimeError):
    pass


async def run_review(raw_args: str, config: RunConfig, *, backend: AgentBackend | None = None, activation=None, repo_root: Path | None = None, benchmark_id: str | None = None) -> Path:
    config.validate()
    context = RunContext.create(raw_args, config, benchmark_id=benchmark_id)
    logger = EventLogger(
        context.root,
        context.state.run_id,
        benchmark_id=benchmark_id,
        max_content_bytes=config.trace_max_content_bytes,
    )
    policy = activation or DeterministicActivationPolicy()
    trace_scope = sdk_trace_context(
        logger,
        config,
        metadata={"acr_raw_args_sha256": context.state.raw_args_sha256},
    )
    trace_scope.__enter__()
    owned_backend: AgentBackend | None = None
    try:
        logger.emit("run.started", stage=Stage.CREATED.value)
        record = context.state.begin(Stage.RESOLVED)
        resolved = resolve_target(raw_args, context=context, repo_root=repo_root)
        if backend is None:
            owned_backend = create_backend(
                config,
                repo_root=resolved.repo_root,
                run_root=context.root,
            )
            backend = owned_backend
        context.state.complete(record, artifact=context.artifact("canonical-input.txt"))
        context.save()

        logger.emit("run.stage", stage=Stage.ACTIVATED.value)
        record = context.state.begin(Stage.ACTIVATED)
        personas = policy.for_paths(resolved.reviewed_paths, repo_root=resolved.repo_root)
        if not personas:
            raise OrchestrationError("no personas activated for the review target")
        context.state.activated_personas = list(personas)
        context.manifest["activated_personas"] = list(personas)
        context.save()
        context.state.complete(record)

        logger.emit("run.stage", stage=Stage.PROMPTS_BUILT.value)
        record = context.state.begin(Stage.PROMPTS_BUILT)
        built = build_prompts(resolved, personas, context=context)
        factory = AgentFactory(config)
        if config.fan_out:
            invocations = {
                persona: factory.persona(built[persona].instructions, built[persona].input, persona=persona)
                for persona in personas
            }
        else:
            compiler = InstructionCompiler()
            instructions = compiler.compile_combined(
                personas,
                disclosure=os.environ.get("ACR_GUIDELINE_DISCLOSURE", "progressive"),
                guideline_root=resolved.repo_root,
            )
            if context is not None:
                context.write_text("artifacts/instructions/combined.txt", instructions.text)
                context.manifest["combined_instruction_hash"] = instructions.sha256
                context.manifest["catalog_digests"] = dict(instructions.catalog_digests)
                context.save()
            combined_invocation = factory.combined(instructions, built[personas[0]].input, personas=personas)
            # Scheduler keeps a uniform persona-keyed input map in both modes;
            # combined mode intentionally points every active key at one run.
            invocations = {persona: combined_invocation for persona in personas}
        context.state.complete(record)
        context.save()

        logger.emit("run.stage", stage=Stage.PASSES_RUNNING.value)
        record = context.state.begin(Stage.PASSES_RUNNING)
        scheduler = PassScheduler(backend, config, logger)
        fragments = await scheduler.run(invocations, personas)
        for persona, value in fragments.items():
            context.write_json(f"artifacts/fragments/{persona}.json", value)
        context.state.complete(record)
        context.save()

        logger.emit("run.stage", stage=Stage.PASSES_VALIDATED.value)
        record = context.state.begin(Stage.PASSES_VALIDATED)
        # assemble_fragments is also the strict fragment validator.
        document = assemble_fragments({**resolved.meta, "date": date.today().isoformat()}, fragments, personas, context=context)
        context.state.complete(record)
        context.save()

        logger.emit("run.stage", stage=Stage.ASSEMBLED.value)
        record = context.state.begin(Stage.ASSEMBLED)
        context.state.complete(record, artifact=context.artifact("assembled.json"))
        context.save()

        logger.emit("run.stage", stage=Stage.VERIFIED.value)
        record = context.state.begin(Stage.VERIFIED)
        document = await verify_document(document, backend, config, logger=logger)
        context.write_json("artifacts/verified.json", document.as_json())
        context.state.complete(record, artifact=context.artifact("verified.json"))
        context.save()

        logger.emit("run.stage", stage=Stage.CONSOLIDATED.value)
        record = context.state.begin(Stage.CONSOLIDATED)
        document = await consolidate_document(document, backend, config, logger=logger)
        context.write_json("artifacts/consolidated.json", document.as_json())
        context.state.complete(record, artifact=context.artifact("consolidated.json"))
        context.save()

        logger.emit("run.stage", stage=Stage.SUMMARIZED.value)
        record = context.state.begin(Stage.SUMMARIZED)
        document = await summarize_document(document, backend, config, logger=logger)
        context.write_text("artifacts/summary.md", document.summary or "")
        context.state.complete(record, artifact=context.artifact("summary.md"))
        context.save()

        logger.emit("run.stage", stage=Stage.PUBLISHED.value)
        record = context.state.begin(Stage.PUBLISHED)
        output = publish(document, resolved.output, repo_root=resolved.repo_root, overwrite=bool(int(resolved.meta.get("overwrite", "0"))) or config.overwrite)
        context.write_text("artifacts/final-review.md", output.read_text(encoding="utf-8"))
        context.state.complete(record, artifact=context.artifact("final-review.md"))
        context.save()
        logger.sdk_trace(
            "workflow.final_output",
            stage=Stage.PUBLISHED.value,
            output_path=str(output),
            output=output.read_text(encoding="utf-8"),
        )
        logger.emit("run.completed", stage=Stage.PUBLISHED.value, output=str(output))
        print_artifact("Review report", output)
        print_artifact("Run log", logger.main_path)
        print_artifact("SDK trace", logger.sdk_trace_path)
        return output
    except Exception as exc:
        code = getattr(exc, "code", None) or type(exc).__name__.upper()
        context.state.fail(code, str(exc))
        context.save()
        logger.emit("run.failed", stage=Stage.FAILED.value, error_code=code, error=str(exc))
        raise
    finally:
        await close_backend(owned_backend)
        trace_scope.__exit__(*sys.exc_info())


def run_review_sync(raw_args: str, config: RunConfig, *, backend: AgentBackend | None = None, activation=None, repo_root: Path | None = None, benchmark_id: str | None = None) -> Path:
    return asyncio.run(run_review(raw_args, config, backend=backend, activation=activation, repo_root=repo_root, benchmark_id=benchmark_id))
