from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from urllib.parse import unquote


class PortabilityTests(unittest.TestCase):
    @staticmethod
    def _init_repo(path: Path) -> None:
        subprocess.run(["git", "init", "-q", str(path)], check=True)
        subprocess.run(
            ["git", "config", "user.email", "acr-test@example.invalid"],
            cwd=path,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "ACR Test"], cwd=path, check=True
        )
        subprocess.run(["git", "add", "."], cwd=path, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", "test fixture"], cwd=path, check=True
        )

    def test_copied_acr_runs_without_parent_skill_files(self):
        source = Path(__file__).resolve().parents[1]
        repo_root = Path(
            subprocess.run(
                ["git", "rev-parse", "--show-toplevel"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            copied = root / "portable" / "acr"
            shutil.copytree(
                source,
                copied,
                ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "*.egg-info"),
            )

            self.assertTrue((copied / "scripts" / "resolve_target.sh").is_file())
            self.assertTrue((copied / "scripts" / "print_guideline.py").is_file())
            self.assertTrue((copied / "spec" / "README.md").is_file())
            self.assertFalse((copied / "tools" / "legacy_scripts").exists())

            env = os.environ.copy()
            env.pop("ACR_GUIDELINE_ROOT", None)
            env.pop("PYTHONPATH", None)
            env["ACR_PYTHON"] = sys.executable
            env["ACR_GUIDELINE_ROOT"] = str(repo_root)
            env["ACR_LOG_ROOT"] = str(root / "logs")

            query = subprocess.run(
                [
                    sys.executable,
                    str(copied / "scripts" / "print_guideline.py"),
                    "catalog",
                    "documentation",
                ],
                cwd=repo_root,
                env=env,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(query.returncode, 0, query.stderr)
            self.assertIn(
                "GUIDELINE_CATALOG persona=documentation", query.stdout
            )

            output = root / "portable-review.md"
            review = subprocess.run(
                [
                    "bash",
                    str(copied / "run.sh"),
                    "files",
                    "README.md",
                    str(output),
                    "--backend=fake",
                    "--overwrite",
                ],
                cwd=repo_root,
                env=env,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(review.returncode, 0, review.stderr)
            self.assertTrue(output.is_file())
            self.assertIn("# Summary", output.read_text(encoding="utf-8"))

    def test_run_sh_uses_its_own_checkout_guidelines_by_default(self):
        acr_root = Path(__file__).resolve().parents[1]
        source_repo = Path(
            subprocess.run(
                ["git", "rev-parse", "--show-toplevel"],
                cwd=acr_root,
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )
        guideline_rel = Path("book/src/to-contribute/coding-guidelines")
        stale_sentinel = "TARGET_STALE_GUIDELINE_SENTINEL"

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            target = root / "target"
            target.mkdir()
            (target / "README.md").write_text("# Target\n", encoding="utf-8")
            shutil.copytree(
                source_repo / guideline_rel,
                target / guideline_rel,
            )
            target_index = target / guideline_rel / "for-documentation" / "README.md"
            with target_index.open("a", encoding="utf-8") as stream:
                stream.write(f"\n{stale_sentinel}\n")
            self._init_repo(target)

            env = os.environ.copy()
            env.pop("ACR_GUIDELINE_ROOT", None)
            env.pop("PYTHONPATH", None)
            env["ACR_PYTHON"] = sys.executable
            env["ACR_LOG_ROOT"] = str(root / "logs")

            output = root / "review.md"
            review = subprocess.run(
                [
                    "bash",
                    str(acr_root / "run.sh"),
                    "files",
                    "README.md",
                    str(output),
                    "--backend=fake",
                    "--overwrite",
                ],
                cwd=target,
                env=env,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(review.returncode, 0, review.stderr)
            instructions = next(
                (root / "logs").glob(
                    "acr-*/artifacts/instructions/documentation.txt"
                )
            ).read_text(encoding="utf-8")
            self.assertNotIn(stale_sentinel, instructions)

    def test_spec_local_links_are_relative_and_resolve(self):
        spec_root = Path(__file__).resolve().parents[1] / "spec"
        link_pattern = re.compile(r"!?\[[^\]]*\]\(([^)]+)\)")

        for document in sorted(spec_root.glob("*.md")):
            with self.subTest(document=document.name):
                for raw_target in link_pattern.findall(
                    document.read_text(encoding="utf-8")
                ):
                    target = raw_target.strip().split(maxsplit=1)[0].strip("<>")
                    if target.startswith(("#", "https://", "http://", "mailto:")):
                        continue
                    path_text = unquote(target.split("#", 1)[0])
                    self.assertFalse(
                        Path(path_text).is_absolute(),
                        f"{document}: local link must be relative: {target}",
                    )
                    self.assertTrue(
                        (document.parent / path_text).exists(),
                        f"{document}: broken local link: {target}",
                    )


if __name__ == "__main__":
    unittest.main()
