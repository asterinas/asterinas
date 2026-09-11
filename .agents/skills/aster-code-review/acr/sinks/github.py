"""Opt-in adapter for the ACR GitHub publishing script."""

from __future__ import annotations

import subprocess
from pathlib import Path


def publish(
    path: Path,
    *,
    repo: str,
    pr: int | str,
    head_sha: str,
    finalize: bool = False,
    event: str = "comment",
) -> None:
    """Publishes a review by forwarding arguments to the established script."""

    script = Path(__file__).resolve().parents[1] / "scripts" / "post_reviews_to_github.sh"
    command = [
        str(script),
        "--repo",
        repo,
        "--pr",
        str(pr),
        "--head-sha",
        head_sha,
        "--event",
        event,
    ]
    if finalize:
        command.append("--finalize")
    command.append(str(path))
    subprocess.run(command, check=True)
