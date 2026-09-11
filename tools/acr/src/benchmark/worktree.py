"""Detached worktree lifecycle used by benchmark runs."""

from __future__ import annotations

import subprocess
from pathlib import Path


class WorktreeError(RuntimeError):
    pass


class WorktreeManager:
    def __init__(self, repo_root: Path, work_root: Path):
        self.repo_root = repo_root.resolve()
        self.work_root = work_root.resolve()
        self.work_root.mkdir(parents=True, exist_ok=True)

    def ensure_commit(self, commit: str, remote: str | None = None) -> None:
        check = subprocess.run(["git", "cat-file", "-e", f"{commit}^{{commit}}"], cwd=self.repo_root, capture_output=True, check=False)
        if check.returncode == 0:
            return
        if not remote:
            raise WorktreeError(f"commit is not available locally: {commit}")
        fetched = subprocess.run(["git", "fetch", "--no-tags", remote, commit], cwd=self.repo_root, capture_output=True, text=True, check=False)
        if fetched.returncode:
            raise WorktreeError(fetched.stderr.strip() or f"cannot fetch commit {commit}")

    def add(self, name: str, commit: str, *, remote: str | None = None) -> Path:
        self.ensure_commit(commit, remote)
        path = self.work_root / name
        if path.exists():
            raise WorktreeError(f"worktree already exists: {path}")
        result = subprocess.run(["git", "worktree", "add", "--detach", str(path), commit], cwd=self.repo_root, capture_output=True, text=True, check=False)
        if result.returncode:
            raise WorktreeError(result.stderr.strip() or "git worktree add failed")
        return path

    def remove(self, path: Path) -> None:
        result = subprocess.run(["git", "worktree", "remove", "--force", str(path)], cwd=self.repo_root, capture_output=True, text=True, check=False)
        if result.returncode:
            raise WorktreeError(result.stderr.strip() or f"cannot remove worktree {path}")
        subprocess.run(["git", "worktree", "prune"], cwd=self.repo_root, capture_output=True, check=False)
